use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_inline(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = FunctionCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let single_use = collector.get_single_use_functions();
    eprintln!("Single use functions: {:?}", single_use.keys().collect::<Vec<_>>());

    if !single_use.is_empty() {
        let mut inliner = FunctionInliner::new(single_use);
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
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
