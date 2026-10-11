use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::rename_webpack_bindings;

fn run(code: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, source_type).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "input must parse: {:?}",
        parsed.diagnostics
    );
    let mut program = parsed.program;
    rename_webpack_bindings(&allocator, &mut program);
    let out = Codegen::new().build(&program).code;
    let reparsed = Parser::new(&allocator, &out, source_type).parse();
    assert!(reparsed.diagnostics.is_empty(), "output must parse: {out}");
    out
}

fn run_script(code: &str) -> String {
    run(code, SourceType::cjs())
}

const WEBPACK5_RUNTIME: &str = r#"(()=>{var t={
    10(e,t,r){r.d(t,{greet:()=>n,Counter:function(){return o},KV:()=>n});function n(){return "hi"}class o{}},
    20:(e,t)=>{t.add=(a,b)=>a+b},
    30:e=>{e.exports=1}
},o={};function r(n){var i=o[n];if(void 0!==i)return i.exports;var a=o[n]={exports:{}};return t[n](a,a.exports,r),a.exports}
r.d=(e,t)=>{for(var n in t)r.o(t,n)&&!r.o(e,n)&&Object.defineProperty(e,n,{enumerable:!0,get:t[n]})};
r.o=(e,t)=>Object.prototype.hasOwnProperty.call(e,t);
(()=>{var e=r(10),n=r(30),i=r.n(n);console.log((0,e.greet)(),i())})()})();"#;

#[test]
fn names_webpack5_runtime_and_factories() {
    let out = run_script(WEBPACK5_RUNTIME);
    assert!(out.contains("function __webpack_require__(moduleId)"), "{out}");
    assert!(out.contains("var __webpack_modules__ = {"), "{out}");
    assert!(out.contains("__webpack_module_cache__ = {}"), "{out}");
    assert!(
        out.contains("__webpack_modules__[moduleId](a, a.exports, __webpack_require__)"),
        "{out}"
    );
    assert!(out.contains("10(module, exports, __webpack_require__)"), "{out}");
    assert!(out.contains("20: (module, exports) =>"), "{out}");
    assert!(out.contains("30: (module) =>"), "{out}");
    assert!(out.contains("module.exports = 1"), "{out}");
    assert!(
        out.contains("__webpack_require__.d = (e, t) =>"),
        "runtime params untouched: {out}"
    );
}

#[test]
fn recovers_export_names_from_define_getters() {
    let out = run_script(WEBPACK5_RUNTIME);
    assert!(out.contains("function greet()"), "{out}");
    assert!(out.contains("class Counter"), "{out}");
    assert!(out.contains("greet: () => greet"), "{out}");
    assert!(out.contains("KV: () => greet"), "first export name wins: {out}");
}

#[test]
fn names_imports_and_interop_getters() {
    let out = run_script(WEBPACK5_RUNTIME);
    assert!(out.contains("var module_10 = __webpack_require__(10)"), "{out}");
    assert!(
        out.contains("module_30_default = __webpack_require__.n(module_30)"),
        "{out}"
    );
    assert!(out.contains("(0, module_10.greet)()"), "{out}");
}

#[test]
fn names_webpack4_bootstrap_array_with_call_invocation() {
    let code = r#"!function(t){var r={};function n(e){if(r[e])return r[e].exports;var o=r[e]={i:e,l:!1,exports:{}};return t[e].call(o.exports,o,o.exports,n),o.l=!0,o.exports}n.d=function(t,r,e){Object.defineProperty(t,r,{enumerable:!0,get:e})};n(n.s=1)}([function(t,r,e){"use strict";e.d(r,"pad",function(){return o});function o(){}},function(t,r,e){var n=e(0);t.exports=n}]);"#;
    let out = run_script(code);
    assert!(out.contains("function __webpack_require__(moduleId)"), "{out}");
    assert!(out.contains("!function(__webpack_modules__)"), "{out}");
    assert!(out.contains("__webpack_modules__[moduleId].call("), "{out}");
    assert!(out.contains("function(module, exports, __webpack_require__)"), "{out}");
    assert!(
        out.contains("__webpack_require__.d(exports, \"pad\", function() {\n\t\treturn pad;"),
        "{out}"
    );
    assert!(out.contains("var module_0 = __webpack_require__(0)"), "{out}");
}

#[test]
fn names_jsonp_chunk_factories_and_chunk_runtime() {
    let code = r#"(self.webpackChunkapp=self.webpackChunkapp||[]).push([[245],{360(r,n,t){t.d(n,["compute",()=>f,"VALUE",0,c]);var a=t(784);function f(){return a}const c=1}},r=>{r.O(0,[1],()=>r(5))}]);"#;
    let out = run_script(code);
    assert!(out.contains("360(module, exports, __webpack_require__)"), "{out}");
    assert!(out.contains("function compute()"), "{out}");
    assert!(out.contains("const VALUE = 1"), "{out}");
    assert!(out.contains("var module_784 = __webpack_require__(784)"), "{out}");
    assert!(out.contains("(__webpack_require__) => {"), "{out}");
}

#[test]
fn names_legacy_callback_chunks() {
    let code = r#"webpackJsonp([0],{12:function(e,t,n){n.d(t,"render",function(){return r});function r(){}}});"#;
    let out = run_script(code);
    assert!(
        out.contains("12: function(module, exports, __webpack_require__)"),
        "{out}"
    );
    assert!(out.contains("function render()"), "{out}");
}

#[test]
fn skips_mangled_and_reserved_export_names() {
    let code = r"self.webpackChunk([1],{1(e,t,n){n.d(t,{A:()=>a,default:()=>b,class:()=>c,Ay:()=>d});var a,b,c,d}});";
    let out = run_script(code);
    assert!(out.contains("var a, b, c, d"), "{out}");
}

#[test]
fn inert_on_non_webpack_input() {
    let code = "var m={1:function(a,b){return a+b}};x.d(t,{foo:()=>a});list.push([1,2]);var a;";
    assert_eq!(run_script(code), run_script_unchanged(code));
}

fn run_script_unchanged(code: &str) -> String {
    let allocator = Allocator::default();
    let program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
    Codegen::new().build(&program).code
}

#[test]
fn free_global_named_like_a_factory_param_is_not_captured() {
    // lodash `isBuffer` inside an ESM module reads the free `exports` global.
    let code = r#"webpackJsonp([0],{1:function(e,t,n){var f="object"==typeof exports&&exports;t.x=f}});"#;
    let out = run_script(code);
    assert!(
        out.contains("function(module, exports$1, __webpack_require__)"),
        "{out}"
    );
    assert!(out.contains("typeof exports && exports"), "{out}");
    assert!(out.contains("exports$1.x = f"), "{out}");
}

#[test]
fn inner_binding_does_not_capture_renamed_param() {
    let code = r"webpackJsonp([0],{1:function(e,t){function g(exports){return t.x+exports}t.g=g}});";
    let out = run_script(code);
    assert!(out.contains("function(module, exports$1)"), "{out}");
    assert!(out.contains("return exports$1.x + exports"), "{out}");
}

#[test]
fn export_name_colliding_with_existing_local_gets_suffix() {
    let code = r"webpackJsonp([0],{1:function(e,t,n){var greet=1;function a(){return greet}n.d(t,{greet:()=>a})}});";
    let out = run_script(code);
    assert!(out.contains("function greet$1()"), "{out}");
    assert!(out.contains("return greet;"), "{out}");
    assert!(out.contains("greet: () => greet$1"), "{out}");
}

#[test]
fn export_name_never_shadows_an_outer_reference() {
    // Renaming `a` to `run` would make `run()` inside `a` call itself.
    let code = r"function run(){}webpackJsonp([0],{1:function(e,t,n){function a(){return run()}n.d(t,{run:()=>a})}});";
    let out = run_script(code);
    assert!(out.contains("function run$1()"), "{out}");
    assert!(out.contains("return run();"), "{out}");
}

#[test]
fn shadowed_inner_params_keep_their_names() {
    let code = r"webpackJsonp([0],{1:function(e,t){t.f=function(e){return e}}});";
    let out = run_script(code);
    assert!(out.contains("exports.f = function(e)"), "{out}");
    assert!(out.contains("return e;"), "{out}");
}

#[test]
fn shorthand_properties_keep_their_keys() {
    let code = r"webpackJsonp([0],{1:function(e,t){t.o={e,t};var{e:x}=t}});";
    let out = run_script(code);
    assert!(out.contains("e: module"), "{out}");
    assert!(out.contains("t: exports"), "{out}");
}

#[test]
fn direct_eval_blocks_renaming() {
    let code = r#"webpackJsonp([0],{1:function(e,t,n){eval("t.x=1")}});"#;
    let out = run_script(code);
    assert!(out.contains("function(e, t, n)"), "{out}");
}

#[test]
fn with_statement_blocks_renaming() {
    let code = r"webpackJsonp([0],{1:function(e,t,n){with(n){t.x=y}}});";
    let out = run_script(code);
    assert!(out.contains("function(e, t, n)"), "{out}");
}

#[test]
fn native_es_export_binding_is_not_renamed() {
    let code =
        r"var a=1;export{a};(self.webpackChunk=self.webpackChunk||[]).push([[1],{1(e,t,n){n.d(t,{alpha:()=>a})}}]);";
    let out = run(code, SourceType::mjs());
    assert!(out.contains("var a = 1"), "{out}");
    assert!(out.contains("alpha: () => a"), "{out}");
}

#[test]
fn factories_with_patterns_or_extra_params_are_skipped() {
    let code = r"webpackJsonp([0],{1:function({a},t){t.x=a},2:function(a,b,c,d){b.x=d}});";
    let out = run_script(code);
    assert!(!out.contains("exports"), "{out}");
}
