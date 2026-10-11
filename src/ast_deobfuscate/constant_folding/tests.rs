#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_fold(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut folder = ConstantFolder::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut folder, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_fold_addition() {
    let output = run_fold("var x = 5 + 10;");
    assert!(output.contains("15"), "Should fold 5 + 10 to 15, got: {output}");
}

#[test]
fn test_fold_multiplication() {
    let output = run_fold("var x = 5 * 16;");
    assert!(output.contains("80"), "Should fold 5 * 16 to 80, got: {output}");
}

#[test]
fn test_fold_subtraction() {
    let output = run_fold("var x = 100 - 30;");
    assert!(output.contains("70"), "Should fold 100 - 30 to 70, got: {output}");
}

#[test]
fn test_fold_division() {
    let output = run_fold("var x = 100 / 5;");
    assert!(output.contains("20"), "Should fold 100 / 5 to 20, got: {output}");
}

#[test]
fn test_fold_bitwise_and() {
    let output = run_fold("var x = 12 & 10;");
    assert!(output.contains('8'), "Should fold 12 & 10 to 8, got: {output}");
}

#[test]
fn test_fold_bitwise_or() {
    let output = run_fold("var x = 12 | 10;");
    assert!(output.contains("14"), "Should fold 12 | 10 to 14, got: {output}");
}

#[test]
fn test_fold_bitwise_xor() {
    let output = run_fold("var x = 12 ^ 10;");
    assert!(output.contains('6'), "Should fold 12 ^ 10 to 6, got: {output}");
}

#[test]
fn test_fold_shift_left() {
    let output = run_fold("var x = 5 << 2;");
    assert!(output.contains("20"), "Should fold 5 << 2 to 20, got: {output}");
}

#[test]
fn test_fold_shift_right() {
    let output = run_fold("var x = 20 >> 2;");
    assert!(output.contains('5'), "Should fold 20 >> 2 to 5, got: {output}");
}

#[test]
fn test_fold_comparison_equal() {
    let output = run_fold("var x = 10 === 10;");
    assert!(output.contains("true"), "Should fold 10 === 10 to true, got: {output}");
}

#[test]
fn test_fold_comparison_not_equal() {
    let output = run_fold("var x = 10 !== 5;");
    assert!(output.contains("true"), "Should fold 10 !== 5 to true, got: {output}");
}

#[test]
fn test_fold_comparison_less_than() {
    let output = run_fold("var x = 5 < 10;");
    assert!(output.contains("true"), "Should fold 5 < 10 to true, got: {output}");
}

#[test]
fn test_fold_comparison_greater_than() {
    let output = run_fold("var x = 10 > 5;");
    assert!(output.contains("true"), "Should fold 10 > 5 to true, got: {output}");
}

#[test]
fn test_fold_logical_and() {
    let output = run_fold("var x = true && false;");
    assert!(
        output.contains("false"),
        "Should fold true && false to false, got: {output}"
    );
}

#[test]
fn test_fold_logical_or() {
    let output = run_fold("var x = true || false;");
    assert!(
        output.contains("true"),
        "Should fold true || false to true, got: {output}"
    );
}

#[test]
fn test_fold_unary_negation() {
    let output = run_fold("var x = -(-42);");
    assert!(output.contains("42"), "Should fold -(-42) to 42, got: {output}");
}

#[test]
fn test_fold_unary_not() {
    let output = run_fold("var x = !false;");
    assert!(output.contains("true"), "Should fold !false to true, got: {output}");
}

#[test]
fn test_fold_chained() {
    let output = run_fold("var x = 2 + 3 * 4;");
    eprintln!("Chained output: {output}");
    assert!(output.contains("14"), "Should fold 2 + 3 * 4 to 14, got: {output}");
}

#[test]
fn test_no_fold_division_by_zero() {
    let output = run_fold("var x = 10 / 0;");
    assert!(output.contains('/'), "Should NOT fold division by zero, got: {output}");
}

#[test]
fn test_no_fold_negative_zero() {
    assert_eq!(run_fold("x = -0;"), "x = -0;\n");
    assert_eq!(run_fold("x = 0 * -1;"), "x = 0 * -1;\n");
    assert_eq!(run_fold("x = -4 % 2;"), "x = -4 % 2;\n");
    assert_eq!(run_fold("x = +-0;"), "x = +-0;\n");
}

#[test]
fn test_no_fold_non_integer_division() {
    assert_eq!(run_fold("x = 7 / 2;"), "x = 7 / 2;\n");
    assert_eq!(run_fold("x = 2 / 4;"), "x = 2 / 4;\n");
    assert_eq!(run_fold("x = 0 / 0;"), "x = 0 / 0;\n");
}

#[test]
fn test_fold_fraction_only_when_no_longer() {
    assert_eq!(run_fold("x = 0 + .5;"), "x = .5;\n");
    assert_eq!(run_fold("x = 1.5 * 2;"), "x = 3;\n");
    assert_eq!(run_fold("x = .1 + .2;"), "x = .1 + .2;\n");
}

#[test]
fn test_fold_remainder_keeps_dividend_sign() {
    assert_eq!(run_fold("x = -7 % 2;"), "x = -1;\n");
}

#[test]
fn test_fold_bitwise_uses_int32() {
    assert_eq!(run_fold("x = 4294967296 | 0;"), "x = 0;\n");
    assert_eq!(run_fold("x = ~4294967296;"), "x = -1;\n");
    assert_eq!(run_fold("x = 1 << 31;"), "x = -2147483648;\n");
    assert_eq!(run_fold("x = -1 >>> 0;"), "x = 4294967295;\n");
    assert_eq!(run_fold("x = 1 << 33;"), "x = 2;\n");
}

#[test]
fn test_no_fold_beyond_safe_integer() {
    assert_eq!(run_fold("x = 9007199254740991 + 2;"), "x = 9007199254740991 + 2;\n");
}

#[test]
fn test_fold_negative_result_stays_valid() {
    assert_eq!(run_fold("x = a - 4 * -1;"), "x = a - -4;\n");
}
