use super::*;
use crate::ast_deobfuscate::semantic;
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_with(code: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;
    let mut collector = TrampolineCollector::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
    let trampolines = collector.into_trampolines();
    if !trampolines.is_empty() {
        let mut inliner = TrampolineInliner::new(trampolines);
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }
    Codegen::new().build(&program).code
}

/// Module code is strict: a plain call passes `this = undefined`.
fn run(code: &str) -> String {
    run_with(code, SourceType::mjs())
}

fn run_sloppy(code: &str) -> String {
    run_with(code, SourceType::cjs())
}

fn assert_kept(out: &str, trampoline: &str) {
    assert!(out.contains(trampoline), "trampoline must be kept: {out}");
    assert!(out.contains(".apply(this"), "trampoline body must be kept: {out}");
}

#[test]
fn inlines_var_trampoline() {
    let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(1, 2);");
    assert!(!out.contains("var xB"), "declaration must be removed: {out}");
    assert!(out.contains("LT(S, [1, 2])"), "got: {out}");
}

#[test]
fn inlines_function_decl_trampoline() {
    let out = run("function jQ() { return JA.apply(this, [Qz, arguments]); } jQ();");
    assert!(!out.contains("function jQ"), "declaration must be removed: {out}");
    assert!(out.contains("JA(Qz, [])"), "got: {out}");
}

#[test]
fn inlines_many_call_sites() {
    let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(a); xB(b, c); xB(...d);");
    assert!(!out.contains("var xB"));
    assert!(out.contains("LT(S, [a])") && out.contains("LT(S, [b, c])") && out.contains("LT(S, [...d])"));
}

#[test]
fn inlines_assignment_trampoline() {
    let out = run("var F59; F59 = function(x) { return LT.apply(this, [S, arguments]); }; F59(a);");
    assert!(out.contains("LT(S, [a])"), "call site must be inlined: {out}");
    assert!(!out.contains("F59 = function"), "assignment must be removed: {out}");
}

#[test]
fn sloppy_trampoline_needs_a_dispatcher_that_ignores_this() {
    let free = "function LT(s, a) { return a[0] + s; }\nvar xB = function() { return LT.apply(this, [S, arguments]); };\nxB(1);";
    let out = run_sloppy(free);
    assert!(out.contains("LT(S, [1])") && !out.contains("var xB"), "got: {out}");

    let reads_this =
        "function LT(s, a) { return this; }\nvar xB = function() { return LT.apply(this, [S, arguments]); };\nxB(1);";
    assert_kept(&run_sloppy(reads_this), "var xB");
    assert_kept(
        &run_sloppy("var xB = function() { return LT.apply(this, [S, arguments]); };\nxB(1);"),
        "var xB",
    );
}

#[test]
fn leaves_trampoline_with_other_references_alone() {
    assert_kept(
        &run("var xB = function() { return LT.apply(this, [S, arguments]); }; o.f = xB; xB(1);"),
        "var xB",
    );
    assert_kept(
        &run("function jQ() { return JA.apply(this, [Qz, arguments]); } o.jQ = jQ; o.jQ(1);"),
        "function jQ",
    );
}

#[test]
fn leaves_rebound_trampoline_alone() {
    assert_kept(
        &run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB = g; xB(1);"),
        "var xB",
    );
    assert_kept(
        &run(
            "var F = function() { return LT.apply(this, [S, arguments]); }; F = function() { return LT.apply(this, [T, arguments]); }; F(1);",
        ),
        "var F",
    );
}

#[test]
fn leaves_call_site_with_shadowed_dispatcher_alone() {
    let out = run(
        "var xB = function() { return LT.apply(this, [S, arguments]); };\nfunction f(LT) { return xB(1); }\nfunction g() { let S = 2; return xB(2); }",
    );
    assert_kept(&out, "var xB");
}

#[test]
fn same_name_in_another_scope_is_not_a_trampoline() {
    let out = run(
        "function f() { function xB() { return LT.apply(this, [S, arguments]); } return xB(1); }\nfunction g(xB) { return xB(2); }",
    );
    assert!(out.contains("LT(S, [1])"), "got: {out}");
    assert!(out.contains("return xB(2)"), "unrelated xB must be kept: {out}");
}

#[test]
fn leaves_non_tramp_functions_alone() {
    let out = run("function f() { return g(1, 2); } f();");
    assert!(out.contains("function f"), "non-tramp must be kept: {out}");
}

#[test]
fn leaves_non_this_apply_alone() {
    let out = run("var x = function() { return g.apply(null, [S, arguments]); }; x();");
    assert!(out.contains("var x"), "must keep (apply arg is null not this): {out}");
}

#[test]
fn leaves_non_arguments_second_arg_alone() {
    let out = run("var x = function() { return g.apply(this, [S, [a,b]]); }; x();");
    assert!(out.contains("var x"), "must keep (2nd arg not 'arguments'): {out}");
}

#[test]
fn preserves_non_trampoline_assignment() {
    let out = run("F59 = function(x) { return x + 1; }; F59(a);");
    assert!(out.contains("F59(a)"), "call site must be unchanged: {out}");
}
