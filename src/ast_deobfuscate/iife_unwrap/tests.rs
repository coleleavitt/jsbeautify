use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_unwrap(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut unwrapper = IifeUnwrap::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
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
fn test_preserve_standalone_iife_with_return_guard() {
    let (output, count) = run_unwrap("(() => { if (!ready()) { return; } start(); })();");
    assert_eq!(
        count, 0,
        "Should NOT unwrap standalone IIFE containing return, got: {count}; output: {output}"
    );
    assert!(output.contains("return"), "Should preserve guarded return: {output}");
    assert!(output.contains("=>"), "Should still contain IIFE arrow, got: {output}");
}

#[test]
fn test_preserve_non_iife_arrow() {
    let (output, count) = run_unwrap("const f = () => { return 1; };");
    assert_eq!(count, 0, "Should NOT unwrap non-IIFE arrow, got: {count}");
    assert!(output.contains("=>"), "Should still contain '=>', got: {output}");
}

fn has_semantic_errors(code: &str) -> bool {
    let allocator = Allocator::default();
    let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    !semantic::build(&program).diagnostics.is_empty()
}

#[test]
fn test_preserve_sibling_iifes_with_colliding_names() {
    // Webpack-runtime shape: two arrow IIFEs each own a `var v2`; splicing
    // both would merge them into one shared binding.
    let src = "(() => { var v2 = 1; f(v2); })(); (() => { var v2 = 2; g(v2); })();";
    let (output, count) = run_unwrap(src);
    assert_eq!(count, 0, "colliding siblings must stay wrapped: {output}");
}

#[test]
fn test_preserve_iife_colliding_with_outer_lexical_binding() {
    let src = "(() => { var obj2 = {}; use(obj2); })(); const obj2 = { a: 1 };";
    let (output, count) = run_unwrap(src);
    assert_eq!(count, 0, "must not redeclare an outer const: {output}");
    assert!(!has_semantic_errors(&output), "output must stay valid: {output}");
}

#[test]
fn test_preserve_iife_shadowing_outer_reference() {
    // Hoisting `x` would capture the sibling statement's reference to the
    // outer/global `x`.
    let (output, count) = run_unwrap("(() => { let x = 1; f(x); })(); g(x);");
    assert_eq!(count, 0, "must not capture an outer reference: {output}");
}

#[test]
fn test_preserve_iife_shadowing_parameter() {
    let (output, count) = run_unwrap("function h(p) { (() => { let p = 1; f(p); })(); }");
    assert_eq!(count, 0, "must not redeclare a parameter: {output}");
}

#[test]
fn test_preserve_var_iife_inside_block() {
    // `var` would escape the loop block and be shared across iterations.
    let src = "for (const i of xs) { (() => { var v = i; fns.push(() => v); })(); }";
    let (output, count) = run_unwrap(src);
    assert_eq!(count, 0, "var must not escape a block: {output}");
}

#[test]
fn test_preserve_iife_name_in_assigned_pattern() {
    let src = "const { a = q } = (() => { let q = 1; return o; })();";
    let (output, count) = run_unwrap(src);
    assert_eq!(count, 0, "pattern default must not capture a hoisted name: {output}");
}

#[test]
fn test_unwrap_non_colliding_iife_with_locals() {
    let (output, count) = run_unwrap("(() => { let fresh = 1; f(fresh); })(); g(other);");
    assert_eq!(count, 1, "non-colliding IIFE should unwrap: {output}");
    assert!(!output.contains("=>"), "Should NOT contain '=>', got: {output}");
}

#[test]
fn test_preserve_iife_with_direct_eval() {
    let (output, count) = run_unwrap("(() => { eval(\"var leak = 1\"); })();");
    assert_eq!(count, 0, "direct eval must keep its scope: {output}");
}

#[test]
fn test_rest_param_iife_is_not_unwrapped() {
    let (_, count) = run_unwrap("((...a) => { a(); })();");
    assert_eq!(count, 0, "rest parameter binds an array inside the arrow");
}
