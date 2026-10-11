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

#[test]
fn proxy_used_as_a_value_is_kept() {
    let output =
        run_call_proxy("function t(a) { return a * 2; }\nfunction p(a) { return t(a); }\nvar api = { p };\np(3);");
    assert!(output.contains("function p"), "got: {output}");
    assert!(output.contains("p(3)"), "got: {output}");
}

#[test]
fn eval_target_is_not_moved_to_the_call_site() {
    let output =
        run_call_proxy("function p(c) { return eval(c); }\nvar o = { m: function() { return p(\"this\"); } };");
    assert!(output.contains("function p"), "got: {output}");
}

#[test]
fn target_bound_differently_at_the_call_site_is_kept() {
    for code in [
        "function p(t) { return t(t); }\np(f);",
        "function p(a) { return t(a); }\nfunction f(t) { return p(1); }",
        "function g() {}\nfunction p(a) { return g(a); }\ng = h;\np(1);",
    ] {
        let output = run_call_proxy(code);
        assert!(output.contains("function p"), "got: {output}");
    }
}

#[test]
fn same_name_in_another_scope_is_untouched() {
    let output =
        run_call_proxy("function f() { function p(a) { return t(a); } return p(1); }\nfunction g(p) { return p(2); }");
    assert!(output.contains("return t(1)"), "got: {output}");
    assert!(output.contains("return p(2)"), "got: {output}");
}
