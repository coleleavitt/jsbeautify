use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_inline(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = FunctionCollector::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let single_use = collector.get_single_use_functions();

    if !single_use.is_empty() {
        let mut inliner = FunctionInliner::new(single_use);
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }

    Codegen::new().build(&program).code
}

#[test]
fn test_inline_simple_wrapper() {
    let output = run_inline("function twice(n) { return n * 2; } var result = twice(10);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function twice"),
        "Function declaration should be removed, got: {output}"
    );
    assert!(
        output.contains("10 * 2") || output.contains("10*2"),
        "Call should be inlined, got: {output}"
    );
}

#[test]
fn test_inline_identity() {
    let output = run_inline("function id(x) { return x; } var y = id(42);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function id"),
        "Function should be removed, got: {output}"
    );
    assert!(output.contains("42"), "Should contain the inlined value, got: {output}");
}

#[test]
fn test_preserve_multi_use() {
    let output = run_inline("function add(a, b) { return a + b; } var x = add(1, 2); var y = add(3, 4);");
    eprintln!("Output: {output}");
    assert!(
        output.contains("function add"),
        "Multi-use function should be preserved, got: {output}"
    );
}

#[test]
fn test_inline_with_constant() {
    let output = run_inline("function getConst() { return 42; } var x = getConst();");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function getConst"),
        "Function should be removed, got: {output}"
    );
    assert!(output.contains("42"), "Should contain the constant, got: {output}");
}

fn assert_untouched(code: &str) {
    let allocator = Allocator::default();
    let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    let expected = Codegen::new().build(&program).code;
    assert_eq!(run_inline(code), expected, "must not inline: {code}");
}

#[test]
fn test_inline_with_free_global_and_const_arg() {
    let output = run_inline("const k = 3; function f(a) { return a * scale; } var y = f(k);");
    assert!(output.contains("y = k * scale"), "got: {output}");
    assert!(!output.contains("function f"), "got: {output}");
}

#[test]
fn test_block_scoped_function_is_not_inlined_out_of_scope() {
    assert_untouched("{ function f() { return f } } { function f() { return f } } console.log(typeof f());");
}

#[test]
fn test_shadowed_free_identifier_blocks_inline() {
    assert_untouched("var K = 1; function f(a) { return a + K; } function g() { var K = 2; return f(3); } g();");
}

#[test]
fn test_impure_or_mutable_arguments_block_inline() {
    assert_untouched("function f(a) { return a * 2; } f(g());");
    assert_untouched("let x = 1; x = 2; function f(a) { return a + 1; } f(x);");
    assert_untouched("function f(a) { return a + 1; } f(x);");
}

#[test]
fn test_other_references_block_inline() {
    assert_untouched("function f(a) { return a; } f(1); h(f);");
    assert_untouched("function f(a) { return a + f; } f(1);");
}

#[test]
fn test_named_function_expression_is_not_a_candidate() {
    assert_untouched("var g = function f(a) { return a; }; f(1);");
}

#[test]
fn test_unsupported_shapes_block_inline() {
    assert_untouched("function f(a, b) { return a; } f(1, 2);");
    assert_untouched("function f(a) { return a + arguments; } f(1);");
    assert_untouched("function f(...a) { return a; } f(1);");
    assert_untouched("function f(a = 1) { return a; } f(2);");
    assert_untouched("function f(a) { return a; } f(1, 2);");
}
