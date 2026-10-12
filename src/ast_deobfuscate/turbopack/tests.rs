use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::rename_turbopack_bindings;

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::cjs();
    let parsed = Parser::new(&allocator, code, source_type).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "input must parse: {:?}",
        parsed.diagnostics
    );
    let mut program = parsed.program;
    rename_turbopack_bindings(&allocator, &mut program);
    let out = Codegen::new().build(&program).code;
    let reparsed = Parser::new(&allocator, &out, source_type).parse();
    assert!(reparsed.diagnostics.is_empty(), "output must parse: {out}");
    out
}

const CHUNK: &str = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,
983,e=>{"use strict";var t=e.i(913777),r=e.r(507698);function n(){return t.x+r.y}let a=1,c=2;e.s(["LoadingSpinner",0,n,"value",()=>a,v=>{a=v},"other",()=>c,"inline",0,function(){}])},
11,12,(e,t,r)=>{let s=e.r(5);t.exports=s},
40,e=>e.a(async(t,s)=>{try{var i=e.i(7);let o=1;e.s(["later",()=>o]);s()}catch(e){s(e)}},!1),
50,e=>{function x(){}e.s(["hoisted",()=>x],51);function y(e){var z=e.i(9);return z}}
]);"#;

#[test]
fn names_factory_parameters() {
    let out = run(CHUNK);
    assert!(out.contains("983,\n\t(__turbopack_context__) => {"), "{out}");
    assert!(out.contains("(__turbopack_context__, module, exports) =>"), "{out}");
    assert!(out.contains("module.exports = module_5"), "{out}");
    assert!(
        out.contains(
            "__turbopack_context__.a(async (__turbopack_handle_async_dependencies__, __turbopack_async_result__) =>"
        ),
        "{out}"
    );
    assert!(
        out.contains("__turbopack_async_result__(e)"),
        "catch parameter untouched: {out}"
    );
}

#[test]
fn names_imports_by_module_id() {
    let out = run(CHUNK);
    assert!(
        out.contains("var module_913777 = __turbopack_context__.i(913777)"),
        "{out}"
    );
    assert!(out.contains("module_507698 = __turbopack_context__.r(507698)"), "{out}");
    assert!(out.contains("return module_913777.x + module_507698.y"), "{out}");
    assert!(out.contains("var module_7 = __turbopack_context__.i(7)"), "{out}");
}

#[test]
fn names_exported_locals_including_hoisted_modules() {
    let out = run(CHUNK);
    assert!(out.contains("function LoadingSpinner()"), "{out}");
    assert!(out.contains("let value = 1, other = 2"), "getter after a setter: {out}");
    assert!(out.contains("\"value\",\n\t\t\t() => value,"), "{out}");
    assert!(out.contains("let later = 1"), "{out}");
    assert!(out.contains("function hoisted()"), "{out}");
    assert!(out.contains("], 51)"), "{out}");
}

#[test]
fn leaves_shadowing_context_names_alone() {
    let out = run(CHUNK);
    assert!(out.contains("function y(e) {\n\t\t\tvar z = e.i(9);"), "{out}");
}

#[test]
fn suffixes_names_that_would_capture_free_references() {
    let code =
        r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c.js",1,(e,t,r)=>{t.exports=typeof module}]);"#;
    let out = run(code);
    assert!(out.contains("(__turbopack_context__, module$1, exports) =>"), "{out}");
    assert!(out.contains("module$1.exports = typeof module"), "{out}");
}

#[test]
fn guards_bindings_visible_to_direct_eval() {
    let code = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c.js",1,e=>{var t=e.i(2);eval("t")}]);"#;
    let out = run(code);
    assert!(out.contains("var t = e.i(2)"), "{out}");
}

#[test]
fn reads_strict_factory_iife_and_computed_global() {
    let code = r#"(self["TURBOPACK"]||(self["TURBOPACK"]=[])).push(["c.js",(()=>{"use strict";return[1,e=>{e.v(1)},2,e=>{}]})(),3,function(e,t){t.exports=1}]);"#;
    let out = run(code);
    assert_eq!(out.matches("(__turbopack_context__) =>").count(), 2, "{out}");
    assert!(out.contains("function(__turbopack_context__, module)"), "{out}");
}

#[test]
fn names_runtime_context_and_installed_functions() {
    let code = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([document.currentScript,{otherChunks:["a.js"],runtimeModuleIds:[1]}]),(()=>{function i(e,t){this.m=e,this.e=t}let u=i.prototype;function b(e){return this.r(e)}function h(e){this.m.exports=e}u.i=b,u.v=h,u.s=function(){},i.prototype.g=globalThis})();"#;
    let out = run(code);
    assert!(out.contains("function Context(e, t)"), "{out}");
    assert!(out.contains("let contextPrototype = Context.prototype"), "{out}");
    assert!(out.contains("function esmImport(e)"), "{out}");
    assert!(out.contains("contextPrototype.v = exportValue"), "{out}");
    assert!(out.contains("Context.prototype.g = globalThis"), "{out}");
}

#[test]
fn ignores_lookalike_pushes() {
    for code in [
        r"queue.push([1,1,e=>{e.i(2)}]);",
        r"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([1,1,e=>{},2]);",
        r"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([1,f,e=>{}]);",
        r"(globalThis.webpackChunk||(globalThis.webpackChunk=[])).push([1,1,e=>{var t=e.i(2)}]);",
    ] {
        let out = run(code);
        assert!(!out.contains("__turbopack_context__"), "{code} → {out}");
    }
}

#[test]
fn names_default_export_locals_after_their_module() {
    let out = run(r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["chunk.js",
        910733,e=>{"use strict";function i(){}i._isSVG=!0,e.s(["default",0,i])},
        50,e=>{function x(){}e.s(["default",()=>x],396713);let n=e.i(7);e.s(["default",0,n])},
        "[project]/app/page.tsx [app-client] (ecmascript)",e=>{let p=1;e.s(["default",0,p,"Page",0,p])}
    ]);"#);
    assert!(out.contains("function default_910733()"), "{out}");
    assert!(out.contains("function default_396713()"), "hoisted module id: {out}");
    assert!(
        out.contains("let module_7 ="),
        "import names win over default names: {out}"
    );
    assert!(out.contains("let Page = 1"), "named exports win: {out}");
}

#[test]
fn names_default_export_locals_from_string_module_ids() {
    let out = run(r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["chunk.js",
        "[project]/app/page.tsx [app-client] (ecmascript)",e=>{let p=1;e.s(["default",0,p])}
    ]);"#);
    assert!(out.contains("let page_default = 1"), "{out}");
}
