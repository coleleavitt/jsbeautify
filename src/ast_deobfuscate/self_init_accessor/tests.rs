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

#[test]
fn declares_the_cache_in_the_body_that_declares_the_accessor() {
    let out = run(
        "function outer() { foo(); function F() { var x = []; F = function () { return x; }; return x; } \
                   return F(); }",
    );
    let body = out.find("function outer").unwrap();
    assert!(
        out[body..].contains("var __F_cache = []"),
        "each run of outer needs its own cache: {out}"
    );
}

#[test]
fn picks_a_cache_name_the_program_does_not_use() {
    let out = run("var __F_cache = 1; function F() { var x = []; F = function () { return x; }; return x; } F();");
    assert!(
        out.contains("var __F_cache_2 = []") && out.contains("return __F_cache_2"),
        "got: {out}"
    );
}

#[test]
fn leaves_accessors_whose_rewrite_could_be_observed() {
    for code in [
        // Kept as a value, so the original function may run again.
        "function F() { var x = []; F = function () { return x; }; return x; } var g = F;",
        // `Object` is a local binding, and `Object.create` keeps its argument.
        "var Object = function () {}; function F() { var x = new Object(); F = function () { return x; }; return x; }",
        "function F() { var x = Object.create(p); F = function () { return x; }; return x; }",
        // Later calls return `x` itself, not a promise.
        "async function F() { var x = []; F = function () { return x; }; return x; }",
        // Reassigned elsewhere, or the getter returns another binding.
        "function F() { var x = []; F = function () { return x; }; return x; } F = null;",
        "function F() { var x = []; F = function (x) { return x; }; return x; }",
        // In a block, a loop body may run it again with another `F`.
        "{ function F() { var x = []; F = function () { return x; }; return x; } }",
    ] {
        let out = run(code);
        assert!(!out.contains("cache"), "{code}\n=> {out}");
    }
}
