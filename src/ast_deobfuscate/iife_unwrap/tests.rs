#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_unwrap(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut unwrapper = IifeUnwrap::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut unwrapper, &mut program, &mut ctx);

    (Codegen::new().build(&program).code, unwrapper.unwrapped_count())
}

#[test]
fn test_unwrap_standalone_iife() {
    let (output, count) = run_unwrap("(() => { doA(); doB(); })();");
    assert!(count >= 1, "Should have unwrapped at least 1 IIFE, got: {count}");
    assert!(output.contains("doA()"), "Should contain 'doA()', got: {output}");
    assert!(output.contains("doB()"), "Should contain 'doB()', got: {output}");
    assert!(!output.contains("=>"), "Should NOT contain '=>', got: {output}");
}

#[test]
fn test_unwrap_assigned_iife_single_return() {
    let (output, count) = run_unwrap("let x = (() => { return 42; })();");
    assert!(count >= 1, "Should have unwrapped at least 1 IIFE, got: {count}");
    assert!(
        output.contains("let x = 42"),
        "Should contain 'let x = 42', got: {output}"
    );
    assert!(!output.contains("=>"), "Should NOT contain '=>', got: {output}");
}

#[test]
fn test_unwrap_assigned_iife_with_body() {
    let (output, count) = run_unwrap("let x = (() => { let q = 1; return q + 2; })();");
    assert!(count >= 1, "Should have unwrapped at least 1 IIFE, got: {count}");
    assert!(
        output.contains("let q = 1"),
        "Should contain 'let q = 1', got: {output}"
    );
    assert!(
        output.contains("let x = q + 2"),
        "Should contain 'let x = q + 2', got: {output}"
    );
    assert!(!output.contains("=>"), "Should NOT contain '=>', got: {output}");
}

#[test]
fn test_preserve_iife_with_args() {
    let (output, count) = run_unwrap("let x = ((a) => { return a + 1; })(5);");
    assert_eq!(count, 0, "Should NOT unwrap IIFE with parameters, got: {count}");
    assert!(output.contains("=>"), "Should still contain '=>', got: {output}");
}

#[test]
fn test_preserve_iife_with_early_return() {
    let (output, count) = run_unwrap("let x = (() => { if (true) return 1; return 2; })();");
    assert_eq!(count, 0, "Should NOT unwrap IIFE with early return, got: {count}");
    assert!(output.contains("=>"), "Should still contain '=>', got: {output}");
}

#[test]
fn test_preserve_non_iife_arrow() {
    let (output, count) = run_unwrap("const f = () => { return 1; };");
    assert_eq!(count, 0, "Should NOT unwrap non-IIFE arrow, got: {count}");
    assert!(output.contains("=>"), "Should still contain '=>', got: {output}");
}
