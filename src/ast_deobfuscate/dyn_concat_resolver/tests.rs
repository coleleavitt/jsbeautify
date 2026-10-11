use super::*;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    run_n(code, 5)
}

fn run_n(code: &str, max_iters: usize) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;
    let mut pass = DynConcatResolver::new();
    for _ in 0..max_iters {
        pass.reset_changed();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        if !pass.changed() {
            break;
        }
    }
    Codegen::new().build(&program).code
}

#[test]
fn inlines_var_into_chain_with_literal_sibling() {
    let out = run("var p = \"/api/v1/\"; var u = p + e + \"/Submit\";");
    assert!(out.contains("\"/api/v1/\""), "got: {out}");
    assert!(
        !out.contains("p +") && !out.contains("p+"),
        "p should be inlined: {out}"
    );
}

#[test]
fn inlines_var_into_chain_without_literal_sibling() {
    let out = run("var e = \"/Trading/Sleeve/\"; fetch(e + d);");
    assert!(out.contains("\"/Trading/Sleeve/\""), "got: {out}");
    assert!(!out.contains("e + d"), "e should be inlined: {out}");
}

#[test]
fn inlines_let_string() {
    let out = run("let c = \"/QueryStudio/Queries/Summarize\"; var u = c + \"-suffix\";");
    assert!(out.contains("\"/QueryStudio/Queries/Summarize-suffix\""), "got: {out}");
}

#[test]
fn inlines_const_string() {
    let out = run("const t = \"/Settings/WebOptions\"; var u = t + e;");
    assert!(out.contains("\"/Settings/WebOptions\""), "got: {out}");
}

#[test]
fn skips_truly_numeric_addition() {
    let out = run("var n = a + b;");
    assert!(
        out.contains("a + b") || out.contains("a+b"),
        "no aliases means no change: {out}"
    );
}

#[test]
fn refuses_to_inline_mutated_ident() {
    let out = run("var p = \"/a/\"; p = \"/b/\"; fetch(p + e);");
    assert!(!out.contains("\"/a/\" + e"), "mutated p must not be inlined: {out}");
}

#[test]
fn handles_compound_assignment_as_mutation() {
    let out = run("var p = \"/a/\"; p += \"x\"; fetch(p + e);");
    assert!(!out.contains("\"/a/\" + e"), "compound assign must poison: {out}");
}

#[test]
fn inlines_into_template_with_identifier() {
    let out = run("const BASE = \"/api/v1\"; var u = `${BASE}/Reporting/${id}`;");
    assert!(out.contains("/api/v1/Reporting/"), "got: {out}");
    assert!(!out.contains("${BASE}"), "BASE must be inlined: {out}");
    assert!(out.contains("${id}"), "unknown id remains: {out}");
}

#[test]
fn escapes_template_raw_when_inlining() {
    let out = run("const PAT = \"['\\\"`] ${x} \\\\ \"; var u = `${PAT}${id}`;");
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &out, SourceType::mjs()).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "output must parse: {out:?} / {:?}",
        parsed.diagnostics
    );
    assert!(out.contains("\\`"), "backtick must be escaped in template raw: {out}");
    assert!(
        out.contains("\\${x}"),
        "template opener must be escaped in template raw: {out}"
    );
}

#[test]
fn inlines_this_property_into_chain() {
    let out = run("class Svc { ctor() { this.BASE_URL = \"/api/\"; } get(id) { return this.BASE_URL + id; } }");
    assert!(out.contains("\"/api/\""), "got: {out}");
}

#[test]
fn inlines_this_property_into_template() {
    let out = run(
        "class Svc { ctor() { this.BASE_URL = \"/Security/Privileges\"; } get(a) { return `${this.BASE_URL}/${a}`; } }",
    );
    assert!(out.contains("/Security/Privileges/"), "got: {out}");
    assert!(
        !out.contains("${this.BASE_URL}"),
        "this.BASE_URL must be inlined: {out}"
    );
}

#[test]
fn inlines_object_property_into_chain() {
    let out = run("o.baseUrl = \"/api/v1\"; fetch(o.baseUrl + \"/users\" + id);");
    assert!(
        out.contains("/api/v1/users"),
        "literal-fold should produce /api/v1/users: {out}"
    );
}

#[test]
fn poisons_object_property_on_reassignment() {
    let out = run("o.baseUrl = \"/api/v1\"; o.baseUrl = \"/api/v2\"; fetch(o.baseUrl + \"/users\");");
    assert!(
        !out.contains("/api/v1/users") && !out.contains("/api/v2/users"),
        "got: {out}"
    );
}

#[test]
fn ignores_computed_member_assignment() {
    let out = run("o[k] = \"/api/v1\"; fetch(o[k] + e);");
    assert!(out.contains("o[k]"), "computed member must be ignored: {out}");
}

#[test]
fn resolves_chained_var_declarations() {
    let out = run("var BASE = \"/api\"; var V1 = BASE + \"/v1\"; fetch(V1 + e);");
    assert!(out.contains("\"/api/v1\""), "V1 should fold to /api/v1: {out}");
}

#[test]
fn ignores_oversized_literal() {
    let huge = "x".repeat(MAX_INLINE_LEN + 10);
    let code = format!("var p = \"{huge}\"; var u = p + \"-\" + e;");
    let out = run(&code);
    assert!(
        out.contains("p +") || out.contains("p+"),
        "oversized must be skipped: {out}"
    );
}

#[test]
fn folds_adjacent_string_literals_after_substitution() {
    let out = run("var a = \"/x/\"; var u = a + \"y\" + e;");
    assert!(out.contains("\"/x/y\""), "adjacent literals should fold: {out}");
}

#[test]
fn handles_paren_wrapped_chain() {
    let out = run("var p = \"/api/\"; fetch((p) + (id));");
    assert!(out.contains("\"/api/\""), "paren-wrapped p should be inlined: {out}");
}

#[test]
fn skips_destructured_binding() {
    let out = run("const { url } = config; fetch(url + e);");
    assert!(out.contains("url"), "destructured binding must not crash: {out}");
}

#[test]
fn respects_inner_scope_shadow() {
    let out =
        run("var BASE = \"/outer/\"; function f() { var BASE = \"/inner/\"; return BASE + x; } var u = BASE + y;");
    assert!(out.contains("\"/outer/\""), "outer BASE inlined for u: {out}");
    assert!(out.contains("\"/inner/\""), "inner BASE inlined inside f: {out}");
}

#[test]
fn deeply_nested_template_literal() {
    let out = run(
        "const BASE_URL_FOR_USER = \"/Authorization/User/Privileges\"; var u = `${BASE_URL_FOR_USER}/${a}?resetCache=${t}`;",
    );
    assert!(out.contains("/Authorization/User/Privileges/"), "got: {out}");
}

#[test]
fn keeps_lone_surrogate_literals_unfolded() {
    for code in [
        "NOOP(x = \"\\ud800\" + \"y\");\n",
        "var s = \"\\udc00\";\nf(s + \"y\");\n",
        "var s = \"a\";\nf(`\\ud800${s}`);\n",
    ] {
        let out = run(code);
        assert!(out.contains("\\ud800") || out.contains("\\udc00"), "{code} -> {out}");
    }
}
