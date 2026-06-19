#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_call_proxy(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = CallProxyCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let single_use = collector.get_single_use_proxies();
    eprintln!("Single use proxies: {:?}", single_use.keys().collect::<Vec<_>>());

    if !single_use.is_empty() {
        let mut inliner = CallProxyInliner::new(single_use);
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }

    Codegen::new().build(&program).code
}

#[test]
fn test_detect_simple_proxy() {
    let output = run_call_proxy("function _0xabc(p) { return _0xdec(p); } var x = _0xabc(123);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function _0xabc"),
        "Proxy function should be removed, got: {output}"
    );
    assert!(
        output.contains("_0xdec(123)"),
        "Call should be inlined to target, got: {output}"
    );
}

#[test]
fn test_multi_param_proxy() {
    let output = run_call_proxy("function _wrap(a, b, c) { return _target(a, b, c); } var result = _wrap(1, 2, 3);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function _wrap"),
        "Proxy function should be removed, got: {output}"
    );
    assert!(
        output.contains("_target(1, 2, 3)") || output.contains("_target(1,2,3)"),
        "Call should be inlined to target, got: {output}"
    );
}

#[test]
fn test_preserve_multi_use_proxy() {
    let output = run_call_proxy("function _wrap(x) { return _target(x); } var a = _wrap(1); var b = _wrap(2);");
    eprintln!("Output: {output}");
    assert!(
        output.contains("function _wrap"),
        "Multi-use proxy should be preserved, got: {output}"
    );
}

#[test]
fn test_no_proxy_wrong_order() {
    let output = run_call_proxy("function _wrap(a, b) { return _target(b, a); } var x = _wrap(1, 2);");
    eprintln!("Output: {output}");
    assert!(
        output.contains("function _wrap"),
        "Non-proxy (wrong param order) should be preserved, got: {output}"
    );
}
