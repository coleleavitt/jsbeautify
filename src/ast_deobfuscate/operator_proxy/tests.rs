use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_operator_proxy(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = OperatorProxyCollector::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let proxies = collector.get_proxies();

    if !proxies.is_empty() {
        let mut inliner = OperatorProxyInliner::new(proxies);
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }

    Codegen::new().build(&program).code
}

#[test]
fn test_detect_add_proxy() {
    let output = run_operator_proxy("function _0xadd(a, b) { return a + b; } var x = _0xadd(5, 10);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function _0xadd"),
        "Proxy function should be removed, got: {output}"
    );
    assert!(
        output.contains("5 + 10") || output.contains("5+10"),
        "Call should be inlined to binary op, got: {output}"
    );
}

#[test]
fn test_detect_multiply_proxy() {
    let output = run_operator_proxy("function _mul(a, b) { return a * b; } var x = _mul(3, 4);");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("function _mul"),
        "Proxy function should be removed, got: {output}"
    );
    assert!(
        output.contains("3 * 4") || output.contains("3*4"),
        "Call should be inlined to binary op, got: {output}"
    );
}

#[test]
fn test_multiple_operators() {
    let output = run_operator_proxy(
        "function _add(a, b) { return a + b; } function _mul(a, b) { return a * b; } var x = _add(1, 2); var y = _mul(3, 4);",
    );
    eprintln!("Output: {output}");
    assert!(
        output.contains("1 + 2") || output.contains("1+2"),
        "Add should be inlined, got: {output}"
    );
    assert!(
        output.contains("3 * 4") || output.contains("3*4"),
        "Mul should be inlined, got: {output}"
    );
}

#[test]
fn test_comparison_operator() {
    let output = run_operator_proxy("function _lt(a, b) { return a < b; } var x = _lt(5, 10);");
    eprintln!("Output: {output}");
    assert!(
        output.contains("5 < 10") || output.contains("5<10"),
        "Call should be inlined to comparison, got: {output}"
    );
}

#[test]
fn proxy_used_as_a_value_is_kept() {
    let output =
        run_operator_proxy("function add(a, b) { return a + b; }\nvar s = [1, 2].reduce(add);\nvar t = add(1, 2);");
    assert!(output.contains("function add"), "got: {output}");
    assert!(output.contains("add(1, 2)"), "got: {output}");
}

#[test]
fn same_name_in_another_scope_is_untouched() {
    let output = run_operator_proxy(
        "function f() { function add(a, b) { return a + b; } return add(1, 2); }\nfunction g(add) { return add(3, 4); }",
    );
    assert!(output.contains("return 1 + 2"), "got: {output}");
    assert!(output.contains("return add(3, 4)"), "got: {output}");
}
