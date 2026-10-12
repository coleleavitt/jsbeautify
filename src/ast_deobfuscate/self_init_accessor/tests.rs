use super::*;
use crate::ast_deobfuscate::semantic;
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;
    let mut pass = SelfInitAccessorFlattener::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
    Codegen::new().build(&program).code
}

#[test]
fn flattens_new_object_pattern() {
    let out = run("function hx() { var x = new Object(); hx = function() { return x; }; return x; }");
    assert!(out.contains("var __hx_cache = new Object"), "got: {out}");
    assert!(out.contains("return __hx_cache"), "got: {out}");
    assert!(!out.contains("hx = function"), "self-reassign must be gone: {out}");
}

#[test]
fn flattens_empty_array_pattern() {
    let out = run("function arr() { var a = []; arr = function() { return a; }; return a; }");
    assert!(out.contains("var __arr_cache = []"));
    assert!(out.contains("return __arr_cache"));
}

#[test]
fn flattens_array_entries_pattern() {
    let out = run("function it() { var i = [].entries(); it = function() { return i; }; return i; }");
    assert!(out.contains("var __it_cache = [].entries()"));
}

#[test]
fn leaves_non_pattern_alone() {
    let out = run("function f() { return 1; }");
    assert!(out.contains("return 1"), "got: {out}");
    assert!(!out.contains("cache"), "got: {out}");
}

#[test]
fn leaves_non_whitelisted_init_alone() {
    let out = run("function g() { var v = sideEffect(); g = function() { return v; }; return v; }");
    assert!(out.contains("sideEffect"), "must preserve side-effect init: {out}");
    assert!(out.contains("g = function"), "must preserve reassignment: {out}");
}
