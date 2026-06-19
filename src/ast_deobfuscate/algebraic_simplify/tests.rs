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

    let mut simplifier = AlgebraicSimplifier::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_self_subtract() {
    let output = run_simplify("var r = x - x;");
    assert!(output.contains("= 0"), "Should simplify x - x to 0, got: {output}");
}

#[test]
fn test_multiply_zero_left() {
    let output = run_simplify("var r = 0 * y;");
    assert!(output.contains("= 0"), "Should simplify 0 * y to 0, got: {output}");
}

#[test]
fn test_multiply_zero_right() {
    let output = run_simplify("var r = x * 0;");
    assert!(output.contains("= 0"), "Should simplify x * 0 to 0, got: {output}");
}

#[test]
fn test_self_divide() {
    let output = run_simplify("var r = z / z;");
    assert!(output.contains("= 1"), "Should simplify z / z to 1, got: {output}");
}

#[test]
fn test_self_modulo() {
    let output = run_simplify("var r = a % a;");
    assert!(output.contains("= 0"), "Should simplify a % a to 0, got: {output}");
}

#[test]
fn test_self_xor() {
    let output = run_simplify("var r = b ^ b;");
    assert!(output.contains("= 0"), "Should simplify b ^ b to 0, got: {output}");
}

#[test]
fn test_add_zero_left() {
    let output = run_simplify("var r = 0 + x;");
    assert!(output.contains("= x"), "Should simplify 0 + x to x, got: {output}");
}

#[test]
fn test_add_zero_right() {
    let output = run_simplify("var r = x + 0;");
    assert!(output.contains("= x"), "Should simplify x + 0 to x, got: {output}");
}

#[test]
fn test_multiply_one_left() {
    let output = run_simplify("var r = 1 * x;");
    assert!(output.contains("= x"), "Should simplify 1 * x to x, got: {output}");
}

#[test]
fn test_multiply_one_right() {
    let output = run_simplify("var r = x * 1;");
    assert!(output.contains("= x"), "Should simplify x * 1 to x, got: {output}");
}

#[test]
fn test_divide_one() {
    let output = run_simplify("var r = x / 1;");
    assert!(output.contains("= x"), "Should simplify x / 1 to x, got: {output}");
}

#[test]
fn test_no_simplification() {
    let output = run_simplify("var r = x - y;");
    assert!(output.contains("x - y"), "Should not simplify x - y, got: {output}");
}
