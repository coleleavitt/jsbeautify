#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_simplify(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut simplifier = ExpressionSimplifier::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

    Codegen::new().build(&program).code
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
