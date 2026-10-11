use super::*;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    run_with(code, SourceType::mjs())
}

fn run_script(code: &str) -> String {
    run_with(code, SourceType::cjs())
}

fn run_with(code: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
    assert!(ret.diagnostics.is_empty(), "{code}: {:?}", ret.diagnostics);
    let mut program = ret.program;
    let mut pass = DynConcatResolver::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
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

#[test]
fn skips_reference_before_declaration() {
    let out = run("f(a + \"!\");\nvar a = \"x\";\n");
    assert_eq!(out, "f(a + \"!\");\nvar a = \"x\";\n");
}

#[test]
fn skips_hoisted_function_declaration_reading_later_var() {
    let code = "function f() {\n\treturn a + \"!\";\n}\nconsole.log(f());\nvar a = \"x\";\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_hoisted_function_declaration_after_const() {
    let code = "g();\nconst a = \"x\";\nfunction g() {\n\treturn a + \"!\";\n}\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_function_declaration_nested_in_hoisted_one() {
    let code = "const a = \"x\";\nfunction g() {\n\tfunction h() {\n\t\treturn a + \"!\";\n\t}\n\treturn h();\n}\n";
    assert_eq!(run(code), code);
}

#[test]
fn inlines_into_closures_created_after_declaration() {
    let out = run("const a = \"x\";\nvar f = () => a + \"!\";\n{\n\tfunction g() {\n\t\treturn a + \"?\";\n\t}\n}\n");
    assert!(out.contains("\"x!\""), "arrow after the const: {out}");
    assert!(out.contains("\"x?\""), "block function hoisted after the const: {out}");
}

#[test]
fn skips_var_declared_in_conditional_block() {
    let code = "if (c) {\n\tvar a = \"x\";\n}\nf(a + \"!\");\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_lexical_declaration_in_switch_case() {
    let code = "switch (k) {\n\tcase 1: const a = \"x\";\n\tcase 2: f(a + \"!\");\n}\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_var_redeclaring_parameter() {
    let code = "function g(x) {\n\tvar x = \"s\";\n\targuments[0] = 5;\n\treturn x + \"!\";\n}\n";
    assert_eq!(run_script(code), code);
}

#[test]
fn skips_member_properties() {
    let code = "var o = {};\nf(o.p + \"!\");\no.p = \"x\";\nf(o.p + \"!\");\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_program_with_direct_eval() {
    let code = "const a = \"x\";\nfunction g() {\n\teval(s);\n\treturn a + \"!\";\n}\n";
    assert_eq!(run(code), code);
}

#[test]
fn skips_program_with_with_statement() {
    let code = "var o = { a: 1 };\nfunction g() {\n\tvar a = \"x\";\n\twith(o) f(a + \"!\");\n}\n";
    assert_eq!(run_script(code), code);
}

#[test]
fn script_top_level_admits_only_const() {
    let out = run_script("var a = \"x\";\nconst b = \"y\";\nf(a + \"!\", b + \"!\");\n");
    assert!(
        out.contains("a + \"!\""),
        "global var can be reassigned via the global object: {out}"
    );
    assert!(out.contains("\"y!\""), "global const is immutable: {out}");
}
