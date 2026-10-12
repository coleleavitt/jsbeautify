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

#[test]
fn names_node_chunk_factories() {
    let code = r#"module.exports=["[project]/index.js [test] (ecmascript)",(t,e,o)=>{let s=t.r(5);e.exports=s}];"#;
    let out = run(code);
    assert!(out.contains("(__turbopack_context__, module, exports) =>"), "{out}");
    assert!(out.contains("module.exports = module_5"), "{out}");
}

#[test]
fn names_strict_wrapped_node_chunk_without_mangled_exports() {
    let code = r#"(()=>{"use strict";module.exports=["[project]/index.js [test] (ecmascript)",t=>{console.log((0,t.i("[project]/lone.js [test] (ecmascript)").f)()),t.s([])},"[project]/lone.js [test] (ecmascript)",t=>{function n(){return"lone"}t.s(["f",0,n])}]})();"#;
    let out = run(code);
    assert_eq!(out.matches("(__turbopack_context__) =>").count(), 2, "{out}");
    assert!(out.contains("function n()"), "mangled key `f` is not a name: {out}");
}

#[test]
fn ignores_node_lookalikes() {
    for code in [
        r"module.exports = [1, function(a, b, c, d) {}];",
        r"function f() { module.exports = [1, (e) => {}]; }",
        r"exports.list = [1, (e) => {}];",
    ] {
        let out = run(code);
        assert!(!out.contains("__turbopack_context__"), "{code} → {out}");
    }
}

#[test]
fn names_development_imports_by_module_path() {
    let code = r#"(globalThis["TURBOPACK"] || (globalThis["TURBOPACK"] = [])).push(["static/chunks/a.js",
"[project]/src/app/page.tsx [app-client] (ecmascript)", ((__turbopack_context__) => {
"use strict";
var __TURBOPACK__imported__module__$5b$project$5d2f$src$2f$foo$2e$ts__$5b$app$2d$client$5d$__$28$ecmascript$29$__ = __turbopack_context__.i("[project]/src/foo.ts [app-client] (ecmascript)");
var react = __turbopack_context__.r("[project]/node_modules/.pnpm/react@19.0.0/node_modules/react/index.js [app-client] (ecmascript)");
(0, __TURBOPACK__imported__module__$5b$project$5d2f$src$2f$foo$2e$ts__$5b$app$2d$client$5d$__$28$ecmascript$29$__.foo)(react);
}),
"[externals]/react [external] (react, cjs)", ((__turbopack_context__, module, exports) => {
const mod = __turbopack_context__.x("react", () => require("react"));
module.exports = mod;
}),
]);"#;
    let out = run(code);
    assert!(
        out.contains(
            r#"var module_src_foo_ts = __turbopack_context__.i("[project]/src/foo.ts [app-client] (ecmascript)")"#
        ),
        "{out}"
    );
    assert!(
        out.contains("(0, module_src_foo_ts.foo)(module_react_index_js)"),
        "{out}"
    );
    assert!(
        out.contains(r#"const external_react = __turbopack_context__.x("react""#),
        "{out}"
    );
}

#[test]
fn reads_a_custom_chunk_loading_global() {
    let code = r#"(globalThis["myApp"] || (globalThis["myApp"] = [])).push(["a.js", 1, (e, t) => {}]);"#;
    let out = run(code);
    assert!(out.contains("(__turbopack_context__, module) =>"), "{out}");
    let bare = r"globalThis.myApp.push([1, 1, (e) => {}]);";
    assert!(
        !run(bare).contains("__turbopack_context__"),
        "a bare global must be named TURBOPACK…"
    );
}

#[test]
fn names_node_runtime_functions() {
    let code = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function a(e){}function b(e){}function p(e){}function q(e,t){}function r(e,t){}c.l=a,c.L=b,c.w=p;module.exports=e=>({m:t=>q(e,t),c:t=>r(e,t)});";
    let out = run(code);
    assert!(out.contains("function loadChunkAsync(e)"), "{out}");
    assert!(out.contains("function loadChunkAsyncByUrl(e)"), "{out}");
    assert!(out.contains("function getOrInstantiateRuntimeModule(e, t)"), "{out}");
    assert!(out.contains("(sourcePath) => ({"), "{out}");
    assert!(out.contains("c: (t) => loadRuntimeChunk(sourcePath, t)"), "{out}");

    let browser = run(r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function a(e){}c.l=a;");
    assert!(browser.contains("function loadChunk(e)"), "{browser}");
}

#[test]
fn names_minified_worker_entrypoint() {
    let code = r#"(function(){function i(o){throw console.error(o),Error(o)}if(typeof self.WorkerGlobalScope>"u")i("Worker entrypoint must be loaded in a worker context");var a=new URL(location.href).searchParams.get("params");var e=JSON.parse(a),f=o=>"string"==typeof e[o]?e[o]:"",r=Array.isArray(e[0])?e[0]:[];Object.assign(self,{TURBOPACK_NEXT_CHUNK_URLS:r,TURBOPACK_ASSET_SUFFIX:f(1),TURBOPACK_CHUNK_BASE_PATH:f(2)}),r.reverse(),importScripts.apply(self,r)})();"#;
    let out = run(code);
    for name in [
        "function abort(o)",
        "var params = JSON.parse(a)",
        "param = (o) =>",
        "chunkUrls = Array.isArray(params[0])",
    ] {
        assert!(out.contains(name), "{name}: {out}");
    }
}

#[test]
fn module_ident_drops_root_layer_and_package_prefix() {
    use super::shapes::module_ident;
    assert_eq!(module_ident("983"), "983");
    assert_eq!(
        module_ident("[project]/src/foo.ts [app-client] (ecmascript)"),
        "src_foo_ts"
    );
    assert_eq!(
        module_ident(
            "[project]/node_modules/.pnpm/next@16.0.0/node_modules/next/dist/client/index.js [app-client] (ecmascript) <locals>"
        ),
        "next_dist_client_index_js_locals"
    );
    assert_eq!(
        module_ident("[project]/a.js { X => \"[project]/b.js <locals>\" } [rsc] (ecmascript) <export foo>"),
        "a_js_export_foo",
        "parts inside a nested id do not count"
    );
    assert_eq!(module_ident("[externals]/react [external] (react, cjs)"), "react");
    assert_eq!(
        module_ident("[project]/app/[slug]/page.tsx [app-rsc] (ecmascript)"),
        "app_slug_page_tsx"
    );
    assert_eq!(module_ident("[turbopack]/"), "anonymous");
}

#[test]
fn reads_a_leading_plain_factory_list() {
    // Next.js 16.4 puts its first factories in a nested array literal.
    let browser = run(
        r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["chunks/a.js",[1,(e,t,r)=>{t.exports=e.r(2)},3,e=>{e.v(1)}],4,e=>{let n=e.i(1)}]);"#,
    );
    assert!(
        browser.contains("(__turbopack_context__, module, exports) =>"),
        "{browser}"
    );
    assert_eq!(browser.matches("(__turbopack_context__) =>").count(), 2, "{browser}");
    assert!(
        browser.contains("let module_1 = __turbopack_context__.i(1)"),
        "{browser}"
    );
    let node = run(
        r"module.exports=[[71029,(a,b,c)=>{c._=1},25556,(a,b,c)=>{a.r(71029)}],79835,function(a){a.n(a.i(25556))}];",
    );
    assert_eq!(
        node.matches("(__turbopack_context__, module, exports) =>").count(),
        2,
        "{node}"
    );
    assert!(node.contains("function(__turbopack_context__)"), "{node}");
}
