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
    let mut collector = TrampolineCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
    let trampolines = collector.into_trampolines();
    if !trampolines.is_empty() {
        let mut inliner = TrampolineInliner::new(trampolines);
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }
    Codegen::new().build(&program).code
}

#[test]
fn inlines_var_trampoline() {
    let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(1, 2);");
    assert!(!out.contains("var xB"), "declaration must be removed: {out}");
    assert!(
        out.contains("LT.call(this, S, [1, 2])") || out.contains("LT.call(this,S,[1,2])"),
        "got: {out}"
    );
}

#[test]
fn inlines_function_decl_trampoline() {
    let out = run("function jQ() { return JA.apply(this, [Qz, arguments]); } jQ();");
    assert!(!out.contains("function jQ"), "declaration must be removed: {out}");
    assert!(
        out.contains("JA.call(this, Qz, [])") || out.contains("JA.call(this,Qz,[])"),
        "got: {out}"
    );
}

#[test]
fn inlines_many_call_sites() {
    let out = run("var xB = function() { return LT.apply(this, [S, arguments]); }; xB(a); xB(b, c); xB();");
    assert!(!out.contains("var xB"));
    let hits = out.matches("LT.call(this").count();
    assert_eq!(hits, 3, "all 3 sites must be rewritten, got: {out}");
}

#[test]
fn leaves_non_tramp_functions_alone() {
    let out = run("function f() { return g(1, 2); } f();");
    assert!(out.contains("function f"), "non-tramp must be kept: {out}");
}

#[test]
fn leaves_non_this_apply_alone() {
    let out = run("var x = function() { return g.apply(null, [S, arguments]); }; x();");
    assert!(out.contains("var x"), "must keep (apply arg is null not this): {out}");
}

#[test]
fn leaves_non_arguments_second_arg_alone() {
    let out = run("var x = function() { return g.apply(this, [S, [a,b]]); }; x();");
    assert!(out.contains("var x"), "must keep (2nd arg not 'arguments'): {out}");
}

#[test]
fn inlines_one_param_trampoline() {
    let out = run("var F = function(x) { return LT.apply(this, [S, arguments]); }; F(a);");
    assert!(!out.contains("var F"), "declaration must be removed: {out}");
    assert!(
        out.contains("LT.call(this, S, [a])") || out.contains("LT.call(this,S,[a])"),
        "got: {out}"
    );
}

#[test]
fn inlines_two_param_trampoline() {
    let out = run("var G = function(a, b) { return LT.apply(this, [S, arguments]); }; G(1, 2);");
    assert!(!out.contains("var G"), "declaration must be removed: {out}");
    assert!(
        out.contains("LT.call(this, S, [1, 2])") || out.contains("LT.call(this,S,[1,2])"),
        "got: {out}"
    );
}

#[test]
fn inlines_assignment_trampoline() {
    let out = run("F59 = function(x) { return LT.apply(this, [S, arguments]); }; F59(a);");
    assert!(
        out.contains("LT.call(this, S, [a])") || out.contains("LT.call(this,S,[a])"),
        "call site must be inlined: {out}"
    );
}

#[test]
fn removes_assignment_declaration() {
    let out = run("F59 = function(x) { return LT.apply(this, [S, arguments]); }; F59(a);");
    assert!(!out.contains("F59 = function"), "assignment must be removed: {out}");
    assert!(!out.contains("F59=function"), "assignment must be removed: {out}");
}

#[test]
fn preserves_non_trampoline_assignment() {
    let out = run("F59 = function(x) { return x + 1; }; F59(a);");
    assert!(
        out.contains("F59") && out.contains("function"),
        "non-trampoline must be kept: {out}"
    );
    assert!(out.contains("F59(a)"), "call site must be unchanged: {out}");
}
