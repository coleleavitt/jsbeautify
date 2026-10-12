#[cfg(test)]
use super::*;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;

    let mut collector = WindowAliasCollector::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let aliases = collector.aliases();
    if aliases.is_empty() {
        return Codegen::new().build(&program).code;
    }

    let mut propagator = WindowAliasPropagator::new(aliases);
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut propagator, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn propagates_window_alias() {
    let out = run("var Q6 = window; var x = Q6.document;");
    assert!(
        out.contains("var x = document"),
        "expected Q6.document → document, got: {out}"
    );
    assert!(!out.contains("Q6.document"), "Q6.document should be gone, got: {out}");
}

#[test]
fn preserves_reassigned_alias() {
    let out = run("var Q6 = window; Q6 = other; var x = Q6.document;");
    assert!(
        out.contains("Q6.document"),
        "reassigned alias should not be propagated, got: {out}"
    );
}

#[test]
fn poisons_compound_assignment_with_global_rhs() {
    // `w += window` computes `w + window` (a number/NaN) — w is NOT a global
    // alias, so `w.document` must stay qualified.
    let out = run("var w = window; w += window; var y = w.document;");
    assert!(
        out.contains("w.document"),
        "compound-assigned var must not propagate: {out}"
    );
}

#[test]
fn poisons_var_redeclaration_to_non_global() {
    // `var` hoisting: both decls share one SymbolId; the later non-global
    // redeclaration cancels the alias.
    let out = run("var X = window; var X = other(); var y = X.document;");
    assert!(out.contains("X.document"), "redeclared var must not propagate: {out}");
}

#[test]
fn poisons_update_expression() {
    let out = run("var X = window; X++; var y = X.document;");
    assert!(out.contains("X.document"), "incremented var must not propagate: {out}");
}

#[test]
fn poisons_for_in_loop_target() {
    let out = run("var X = window; for (X in obj) {} var y = X.document;");
    assert!(out.contains("X.document"), "for-in loop var must not propagate: {out}");
}

#[test]
fn poisons_for_of_loop_target() {
    let out = run("var X = window; for (X of arr) {} var y = X.document;");
    assert!(out.contains("X.document"), "for-of loop var must not propagate: {out}");
}

#[test]
fn preserves_computed_access() {
    let out = run("var Q6 = window; var x = Q6[\"doc\"];");
    assert!(
        out.contains("Q6[") || out.contains("Q6."),
        "computed access should not be rewritten, got: {out}"
    );
}

#[test]
fn does_not_clobber_same_named_binding_in_other_scope() {
    // A `var props = window` in one function must NOT cause an unrelated
    // function's `props` PARAMETER to have its `props.in` stripped to the
    // reserved word `in` (which would crash the re-parse checkpoint). This is
    // the core SymbolId-scoping guarantee.
    let code = "function a(){ var props = window; return props.foo; } \
                    function b(props){ if (props.in) { return props.enter; } }";
    let out = run(code);
    assert!(out.contains("props.in"), "param props.in must survive: {out}");
    assert!(out.contains("props.enter"), "param props.enter must survive: {out}");
    assert!(!out.contains("if (in)"), "must never emit bare reserved word: {out}");
}

#[test]
fn does_not_alias_this_as_global() {
    // `var self = this; self.value` reads an instance property — it must NOT
    // become a bare global `value`. `this` is not a global object.
    let out = run("class C { m(){ var self = this; return self.value; } }");
    assert!(
        out.contains("self.value"),
        "this-alias receiver must be preserved: {out}"
    );
    assert!(!out.contains("return value"), "must not strip the this receiver: {out}");
}

#[test]
fn handles_nested_member_access_on_alias_without_panicking() {
    // `alias.a.b`: the inner `alias.a` is rewritten to a bare `a` whose
    // reference_id is unset; visiting the outer member expression must not
    // unwrap that missing id (regression for a get_id.rs unwrap panic).
    let out = run("var Q6 = window; var x = Q6.location.href;");
    assert!(out.contains("location.href"), "nested access rewritten: {out}");
    assert!(!out.contains("Q6.location"), "alias object should be gone: {out}");
}

#[test]
fn does_not_emit_reserved_word_even_for_real_alias() {
    // Even when `props` IS the alias, `props.in` cannot become bare `in`.
    let out = run("var props = window; if (props.in) { x(); }");
    assert!(out.contains("props.in"), "reserved property must stay qualified: {out}");
    assert!(!out.contains("if (in)"), "must not emit bare reserved word: {out}");
}

#[test]
fn does_not_emit_strict_mode_reserved_words() {
    // MJS is always strict, so `async`/`implements`/etc. are illegal as bare
    // identifier references even though they're valid property names.
    for kw in [
        "async",
        "implements",
        "interface",
        "package",
        "private",
        "protected",
        "public",
        "static",
        "let",
        "await",
        "yield",
        "in",
        "for",
        "class",
        "return",
    ] {
        let out = run(&format!("var w = window; var r = w.{kw};"));
        assert!(
            out.contains(&format!("w.{kw}")),
            "{kw}: reserved word must stay qualified, got: {out}"
        );
    }
}

#[test]
fn does_not_rewrite_delete_operand() {
    // `delete Q.document` → `delete document` is a SyntaxError in strict mode.
    // Uses an allowlisted prop so this exercises the delete guard, not the allowlist.
    let out = run("var Q = window; delete Q.document;");
    assert!(
        out.contains("delete Q.document"),
        "delete operand must stay qualified: {out}"
    );
    assert!(
        !out.contains("delete document"),
        "must not emit bare `delete document`: {out}"
    );
}

#[test]
fn does_not_rewrite_parenthesized_delete_operand() {
    // oxc preserves parens, so `delete (Q.document)` wraps the member in a
    // ParenthesizedExpression — the guard must look THROUGH the parens or it
    // emits `delete document` (the same strict-mode SyntaxError).
    let out = run("var Q = window; delete (Q.document);");
    assert!(
        !out.contains("delete document"),
        "paren-delete must not emit bare delete: {out}"
    );
    assert!(
        out.contains("Q.document"),
        "paren-delete operand must stay qualified: {out}"
    );
}

#[test]
fn still_rewrites_member_outside_delete() {
    // A normal (non-delete) member access on the same alias still propagates.
    let out = run("var Q = window; delete Q.document; var x = Q.location;");
    assert!(out.contains("delete Q.document"), "delete operand stays: {out}");
    assert!(out.contains("var x = location"), "normal access still rewritten: {out}");
}

#[test]
fn does_not_capture_local_binding() {
    // `Q.location` → bare `location` would read the LOCAL `location`, not
    // `window.location`. With a local in scope the access must stay qualified.
    let out = run("function f(){ var location = 123; var Q = window; return Q.location; }");
    assert!(out.contains("Q.location"), "must not capture local `location`: {out}");
    assert!(
        !out.contains("return location;"),
        "must not strip to bare local read: {out}"
    );
}

#[test]
fn does_not_capture_parameter_binding() {
    let out = run("function f(navigator){ var Q = window; return Q.navigator; }");
    assert!(out.contains("Q.navigator"), "must not capture param `navigator`: {out}");
}

#[test]
fn still_rewrites_when_no_local_shadows() {
    // No local `document` in scope → the rewrite is sound and proceeds.
    let out = run("function f(){ var Q = window; return Q.document; }");
    assert!(
        out.contains("return document"),
        "unshadowed global still rewritten: {out}"
    );
    assert!(!out.contains("Q.document"), "alias should be gone: {out}");
}

#[test]
fn does_not_rewrite_unknown_global_property() {
    // `window.foo` reads `undefined` for an absent property, but bare `foo`
    // throws ReferenceError. A non-allowlisted property must stay qualified.
    let out = run("var Q = window; var r = Q.foo; var s = Q.someRandomProp;");
    assert!(out.contains("Q.foo"), "unknown property must stay qualified: {out}");
    assert!(
        out.contains("Q.someRandomProp"),
        "unknown property must stay qualified: {out}"
    );
    assert!(!out.contains("= foo"), "must not emit bare unknown global: {out}");
}

#[test]
fn rewrites_known_spec_and_browser_globals() {
    // Allowlisted globals (spec + browser) are safe to strip.
    for prop in ["document", "location", "console", "JSON", "Math", "Promise", "crypto"] {
        let out = run(&format!("var Q = window; var r = Q.{prop};"));
        assert!(out.contains(&format!("var r = {prop}")), "{prop} should rewrite: {out}");
        assert!(
            !out.contains(&format!("Q.{prop}")),
            "{prop}: alias should be gone: {out}"
        );
    }
}

#[test]
fn local_named_window_is_not_the_global_object() {
    let out = run("function f(window) { var w = window; return w.console; }");
    assert!(out.contains("w.console"), "got: {out}");
    let out = run("var self = {};\nvar s = self;\nexport const c = s.console;");
    assert!(out.contains("s.console"), "got: {out}");
}

#[test]
fn keeps_names_the_aliased_global_may_lack() {
    // A worker's `self` has no `document`, Node's `global` no `navigator`, and
    // `globalThis` may be either; bare names would throw where the property
    // read yields `undefined`.
    let out = run("var s = self; var a = s.document, b = s.window, c = s.postMessage;");
    assert!(
        out.contains("s.document") && out.contains("s.window") && out.contains("s.postMessage"),
        "{out}"
    );
    let out = run("var g = global; var a = g.navigator, b = g.document;");
    assert!(out.contains("g.navigator") && out.contains("g.document"), "{out}");
    let out = run("var g = globalThis; var a = g.document, b = g.addEventListener;");
    assert!(
        out.contains("g.document") && out.contains("g.addEventListener"),
        "{out}"
    );
}

#[test]
fn keeps_feature_detected_globals_qualified() {
    let out = run(
        "var w = window; var a = w.caches, b = w.Notification, c = w.fetch, d = w.SharedArrayBuffer, e = w.BigInt, f = w.structuredClone;",
    );
    for kept in [
        "w.caches",
        "w.Notification",
        "w.fetch",
        "w.SharedArrayBuffer",
        "w.BigInt",
        "w.structuredClone",
    ] {
        assert!(out.contains(kept), "{kept}: {out}");
    }
}

#[test]
fn rewrites_names_every_aliased_global_defines() {
    let out = run("var s = self; s.addEventListener(\"message\", f); var p = s.Promise;");
    assert!(
        out.contains("addEventListener(\"message\", f)") && out.contains("var p = Promise"),
        "{out}"
    );
    let out = run("var g = global; var a = g.process, b = g.setTimeout;");
    assert!(
        out.contains("var a = process") && out.contains("b = setTimeout"),
        "{out}"
    );
    let out = run("var w = window; var c = w.crypto ? w.crypto : w.msCrypto;");
    assert!(out.contains("crypto ? crypto : w.msCrypto"), "{out}");
}

#[test]
fn an_alias_of_window_and_self_gets_the_names_both_define() {
    let out = run("var w = window; w = self; var a = w.document, b = w.navigator;");
    assert!(out.contains("w.document") && out.contains("b = navigator"), "{out}");
}
