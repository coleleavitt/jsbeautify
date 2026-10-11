#[cfg(test)]
use super::*;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;
    let mut collector = MethodCallForwarderCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
    let fwds = collector.into_forwarders();
    if !fwds.is_empty() {
        let mut inliner = MethodCallForwarderInliner::new(fwds);
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }
    Codegen::new().build(&program).code
}

#[test]
fn inlines_fn_decl_charcodeat() {
    let out = run("function J(a, b) { return a.charCodeAt(b); } var r = J(s, 3);");
    assert!(out.contains("s.charCodeAt(3)"), "got: {out}");
    assert!(!out.contains("function J"), "forwarder must be removed: {out}");
}

#[test]
fn inlines_var_method_forwarder() {
    let out = run("var idx = function(a, b) { return a.indexOf(b); }; var r = idx(arr, x);");
    assert!(out.contains("arr.indexOf(x)"), "got: {out}");
    assert!(!out.contains("var idx"), "declaration must be removed: {out}");
}

#[test]
fn leaves_swapped_param_alone() {
    let out = run("function S(a, b) { return b.charCodeAt(a); } S(1, s);");
    assert!(out.contains("function S"), "must keep (params swapped): {out}");
}

#[test]
fn leaves_multi_arg_method_alone() {
    let out = run("function M(a, b) { return a.substring(0, b); } M(s, 5);");
    assert!(out.contains("function M"), "must keep (2 method args): {out}");
}

#[test]
fn leaves_single_param_alone() {
    let out = run("function U(a) { return a.toUpperCase(); } U(s);");
    assert!(
        out.contains("function U"),
        "must keep (1-param is not our pattern): {out}"
    );
}

#[test]
fn leaves_non_ident_receiver_alone() {
    let out = run("function K(a, b) { return other.charCodeAt(b); } K(x, 2);");
    assert!(out.contains("function K"), "must keep (receiver is not param0): {out}");
}

#[test]
fn preserves_receiver_expression_at_call_site() {
    let out = run("function J(a, b) { return a.charCodeAt(b); } var r = J(getStr(), i + 1);");
    assert!(
        out.contains("getStr().charCodeAt(i + 1)") || out.contains("getStr().charCodeAt(i+1)"),
        "got: {out}"
    );
}

#[test]
fn forwarder_used_as_a_value_or_with_defaults_is_kept() {
    let out = run("function f(a, b) { return a.push(b); }\nvar g = f;\nf(x, 1);");
    assert!(out.contains("function f") && out.contains("f(x, 1)"), "got: {out}");
    let out = run("function f(a, b = 2) { return a.push(b); }\nf(x, undefined);");
    assert!(
        out.contains("function f") && out.contains("f(x, undefined)"),
        "got: {out}"
    );
}

#[test]
fn same_name_in_another_scope_is_untouched() {
    let out = run(
        "function f() { function p(a, b) { return a.push(b); } return p(x, 1); }\nfunction g(p) { return p(y, 2); }",
    );
    assert!(out.contains("x.push(1)"), "got: {out}");
    assert!(out.contains("return p(y, 2)"), "got: {out}");
}
