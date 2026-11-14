use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_simplify(code: &str) -> String {
    run_measured(code).0
}

/// The output, and the arena bytes the pass allocated.
fn run_measured(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut simplifier = ExpressionSimplifier::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    let before = allocator.used_bytes();
    traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);
    let growth = allocator.used_bytes() - before;

    (Codegen::new().build(&program).code, growth)
}

#[test]
fn test_not_zero_to_true() {
    let output = run_simplify("var x = !0;");
    assert!(output.contains("true"), "Should convert !0 to true, got: {output}");
}

#[test]
fn test_not_one_to_false() {
    let output = run_simplify("var x = !1;");
    assert!(output.contains("false"), "Should convert !1 to false, got: {output}");
}

#[test]
fn test_double_not_array_to_true() {
    let output = run_simplify("var x = !![];");
    assert!(output.contains("true"), "Should convert !![] to true, got: {output}");
}

#[test]
fn test_double_not_zero_to_false() {
    let output = run_simplify("var x = !!0;");
    assert!(output.contains("false"), "Should convert !!0 to false, got: {output}");
}

#[test]
fn test_plus_array_to_zero() {
    let output = run_simplify("var x = +[];");
    assert!(output.contains('0'), "Should convert +[] to 0, got: {output}");
}

#[test]
fn test_plus_not_array_to_one() {
    let output = run_simplify("var x = +![];");
    assert!(output.contains('1'), "Should convert +![] to 1, got: {output}");
}

#[test]
fn test_void_kept() {
    assert_eq!(run_simplify("x = void f();"), "x = void f();\n");
}

#[test]
fn test_bracket_to_dot_notation() {
    let output = run_simplify("obj[\"property\"];");
    assert!(
        output.contains(".property"),
        "Should convert bracket to dot notation, got: {output}"
    );
}

#[test]
fn test_preserve_invalid_identifiers() {
    let output = run_simplify("obj[\"some-property\"];");
    assert!(
        output.contains("[\"some-property\"]"),
        "Should preserve bracket notation for invalid identifiers, got: {output}"
    );
}

#[test]
fn test_string_concat() {
    let output = run_simplify("var x = \"Hel\" + \"lo\";");
    assert!(output.contains("Hello"), "Should concatenate strings, got: {output}");
}

#[test]
fn test_remove_debugger() {
    let output = run_simplify("var x = 1; debugger; var y = 2;");
    assert!(
        !output.contains("debugger"),
        "Should remove debugger statement, got: {output}"
    );
}

#[test]
fn test_string_concat_keeps_lone_surrogate() {
    let output = run_simplify("x = '\\ud800' + 'y';");
    assert!(output.contains("\\ud800"), "lone surrogate must survive, got: {output}");
}

#[test]
fn folds_a_string_chain_into_one_literal() {
    assert_eq!(run_simplify(r#"x = "a" + "b" + "c" + "d";"#), "x = \"abcd\";\n");
}

#[test]
fn folds_only_the_literal_prefix_of_a_chain() {
    assert_eq!(
        run_simplify(r#"x = "a" + "b" + y + "c" + "d"; z = y + "a" + "b";"#),
        "x = \"ab\" + y + \"c\" + \"d\";\nz = y + \"a\" + \"b\";\n"
    );
}

#[test]
fn folds_parenthesized_string_chains_separately() {
    assert_eq!(
        run_simplify(r#"x = ("a" + "b") + "c"; y = "a" + ("b" + "c");"#),
        "x = \"ab\" + \"c\";\ny = \"a\" + \"bc\";\n"
    );
}

/// Folding each `+` of `"a" + "a" + ...` allocated every prefix of the result:
/// 32,000 operands took 500 MB.
#[test]
fn long_string_chain_allocates_linearly() {
    let code = format!("x = \"a\"{};", " + \"a\"".repeat(10_000));
    let (out, growth) = crate::stack::run_on_sized_stack(code.len(), || run_measured(&code));
    assert!(
        growth <= 4 * code.len(),
        "{growth} bytes for {} source bytes",
        code.len()
    );
    assert_eq!(out, format!("x = \"{}\";\n", "a".repeat(10_001)));
}
