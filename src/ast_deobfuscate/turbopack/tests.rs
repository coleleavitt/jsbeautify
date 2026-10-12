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

#[test]
fn names_mangled_exports_from_reexports_in_the_same_file() {
    let out = run(r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c.js",
        13477,e=>{let t=(0,e.i(42871).f)("work-async-storage");e.s(["f",0,t])},
        26088,e=>{function t(){}function n(){}e.s(["a",0,t,"U",0,n])},
        52482,e=>{e.S([96154,0,13477,"workAsyncStorage,f"])},
        16448,e=>{var n=e.i(26088);e.S([n,"getCacheSignal","a"])},
        7,e=>{function t(){}e.s(["b",0,t])},
        8,e=>{e.S([7,"one,b",0,9,"two,b"]);e.S([7,"three,b"])}
    ]);"#);
    assert!(out.contains("let workAsyncStorage = (0, "), "{out}");
    assert!(out.contains("function getCacheSignal()"), "namespace head: {out}");
    assert!(out.contains("function n()"), "key nobody re-exports: {out}");
    assert!(out.contains("function t()"), "two names for one key: {out}");
}

#[test]
fn skips_mangled_reexport_names() {
    let out = run(r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c.js",
        1,e=>{let t=1;e.s(["f",0,t])},
        2,e=>{e.S([1,"a,f"])}
    ]);"#);
    assert!(out.contains("let t = 1"), "{out}");
}

#[test]
fn names_react_refresh_handles() {
    let out = run(
        r#"(globalThis["TURBOPACK"] || (globalThis["TURBOPACK"] = [])).push(["c.js",
"[project]/src/app/counter.tsx [app-client] (ecmascript)", ((__turbopack_context__) => {
"use strict";
var _s = __turbopack_context__.k.signature();
function Counter() {
    _s();
    return 1;
}
_s(Counter, "abc123=");
_c = Counter;
var _c;
__turbopack_context__.k.register(_c, "Counter");
if (typeof globalThis.$RefreshHelpers$ === 'object' && globalThis.$RefreshHelpers !== null) {
    __turbopack_context__.k.registerExports(__turbopack_context__.m, globalThis.$RefreshHelpers$);
}
}),
]);"#,
    );
    assert!(
        out.contains("var refreshSig_Counter = __turbopack_context__.k.signature();"),
        "{out}"
    );
    assert!(out.contains("refreshSig_Counter(Counter, \"abc123=\");"), "{out}");
    assert!(out.contains("refreshReg_Counter = Counter;"), "{out}");
    assert!(
        out.contains("__turbopack_context__.k.register(refreshReg_Counter, \"Counter\");"),
        "{out}"
    );
}

#[test]
fn names_require_context_maps() {
    let out = run(r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c.js",
        60168,e=>{var t=e.f(e.r(52812));e.s(["iconNames",0,function(){return t.keys()}])},
        52812,function(e){e.v({"./moon.js":{id:()=>3456,module:()=>e.r(3456)}})},
        "[project]/src/lib/icons.js [app-rsc] (ecmascript)",(__turbopack_context__)=>{const iconsCtx=__turbopack_context__.f(__turbopack_context__.r("[project]/src/lib/icons.js (require.context ./icons/*)"));const c=__turbopack_context__.f(__turbopack_context__.r("[project]/src/lib/icons.js (require.context ./icons/*)"))}
    ]);"#);
    assert!(
        out.contains("var requireContext_52812 = __turbopack_context__.f("),
        "{out}"
    );
    assert!(out.contains("const iconsCtx ="), "authored names stay: {out}");
    assert!(out.contains("const requireContext_src_lib_icons_js ="), "{out}");
}

#[test]
fn names_the_next_edge_wrapper() {
    let out = run(
        r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["chunks/a.js",
        28517,(e,t,n)=>{self._ENTRIES||={};let s=Promise.resolve().then(()=>e.i(55658));s.catch(()=>{}),self._ENTRIES["middleware_app/api/edge/route"]=new Proxy(s,{get(e,t){if("then"===t)return(t,n)=>e.then(t,n);let n=(...n)=>e.then(e=>(0,e[t])(...n));return n.then=(n,s)=>e.then(e=>e[t]).then(n,s),n}})}
    ]);"#,
    );
    assert!(out.contains("let modProm = Promise.resolve()"), "{out}");
    assert!(out.contains("new Proxy(modProm, {"), "{out}");
    assert!(out.contains("get(innerModProm, name) {"), "{out}");
}

/// The shapes of `browser/runtime/base/dev-base.ts` and `dom/*-backend-dom.ts`
/// after minification.
const MINIFIED_DEV_RUNTIME: &str = r#"(()=>{let e,D;var r=globalThis.TURBOPACK;if(void 0===r)r=[];else if(!Array.isArray(r))return;globalThis.TURBOPACK_CHUNK_UPDATE_LISTENERS||=[];var U={push:e=>globalThis.TURBOPACK_CHUNK_UPDATE_LISTENERS.push(e)};
function i(e,t){this.m=e,this.e=t}let u=i.prototype;function a(e){return this.r(e)}function m(e){}u.i=a;
let L=new Map;u.c=L;u.l=m;
class E extends Error{name="UpdateApplyError";constructor(e){super(e)}}
function k(e,t,r){i.call(this,e,t),this.k=r}k.prototype=i.prototype;
function F(e){throw Error(`Invariant: ${e}`)}
function C(e,t){if(e.type!=="x")throw Error(`Unknown chunk update type: ${e.type}`);return t}
function S(e,t){let r=e.get(t);r.then(()=>{},n=>{throw Error(`Failed to load chunk ${t} ${n}${""}`)});return r}
function W(e){throw Error(function(e){return`Module ${e} was instantiated ${e}, but the module factory is not available.`}(e))}
function X(e){if("function"==typeof globalThis.$RefreshInterceptModuleExecution$){let t=globalThis.$RefreshInterceptModuleExecution$(e.id);try{e.run({register:globalThis.$RefreshReg$,signature:globalThis.$RefreshSig$,registerExports:Y})}finally{t()}}else e.run(Z)}
function Y(e,t){}const Z={register:(e,t)=>{},signature:()=>e=>{},registerExports:(e,t)=>{}};
function H(t){return e.registerChunk(t[0],t[1])}function G(t){}
e={async registerChunk(e,t){},loadChunkCached:(e,t)=>S(e,t)};D={unloadChunk(e){},reloadChunk(e){}};
globalThis.TURBOPACK={push:H},r.forEach(H);var V=globalThis.TURBOPACK_CHUNK_LISTS||[];globalThis.TURBOPACK_CHUNK_LISTS={push:G},V.forEach(G)})();"#;

#[test]
fn names_minified_development_browser_runtime() {
    let out = run(MINIFIED_DEV_RUNTIME);
    for expected in [
        "function Context(e, t)",
        "function DevContext(e, t, refresh)",
        "this.k = refresh",
        "contextPrototype.c = devModuleCache",
        "contextPrototype.l = loadChunk",
        "class UpdateApplyError extends Error",
        "function invariant(e)",
        "function applyChunkListUpdate(e, t)",
        "function loadChunkByUrlWhole(e, t)",
        "function runModuleExecutionHooks(e)",
        "registerExports: registerExportsAndSetupBoundaryForReactRefresh",
        "const DUMMY_REFRESH_CONTEXT = {",
        "function registerChunk(t)",
        "function registerChunkList(t)",
        "chunksToRegister.forEach(registerChunk)",
        "chunkListsToRegister.forEach(registerChunkList)",
        "BACKEND = {",
        "DEV_BACKEND = {",
        "var CHUNK_UPDATE_LISTENERS = {",
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
    assert!(
        out.contains("function W(e)"),
        "a message inlined from another function is not this one's: {out}"
    );
}

#[test]
fn names_development_node_runtime_overrides() {
    let code = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;let M=new Map;function a(e){}function b(e){}c.l=a,c.w=1;c.c=M;let h=globalThis.__turbopack_server_hmr_handlers__??new Map;function o(e){}c.l=o;";
    let out = run(code);
    assert!(out.contains("function loadChunkAsync(e)"), "{out}");
    assert!(out.contains("function loadChunkAsyncOnDemand(e)"), "{out}");
    assert!(out.contains("let devModuleCache = new Map()"), "{out}");
    let prod = run(r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;let M=new Map;c.c=M;");
    assert!(prod.contains("let moduleCache = new Map()"), "{prod}");
}

#[test]
fn runtime_anchors_need_the_runtime() {
    let out = run(r"function F(e){throw Error(`Invariant: ${e}`)}class E extends Error{name='UpdateApplyError'}");
    assert!(out.contains("function F(e)"), "{out}");
    assert!(out.contains("class E extends Error"), "{out}");
}

#[test]
fn keeps_authored_runtime_names() {
    let code = r"function Context(module, exports) { this.m = module; this.e = exports; } const browserContextPrototype = Context.prototype; let devModuleCache = new Map(); browserContextPrototype.c = devModuleCache; function loadChunkInternal(e) {} browserContextPrototype.l = loadChunkInternal;";
    let out = run(code);
    assert!(
        out.contains("const browserContextPrototype = Context.prototype"),
        "{out}"
    );
    assert!(out.contains("let devModuleCache = new Map()"), "{out}");
    assert!(out.contains("function loadChunkInternal(e)"), "{out}");
}

/// `-d`'s Phase 0.86 on a classic script, returning the output and the
/// source type the program ends with.
fn run_script(code: &str) -> (String, SourceType) {
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, SourceType::script()).parse().program;
    rename_turbopack_bindings(&allocator, &mut program);
    let out = Codegen::new().build(&program).code;
    let reparsed = Parser::new(&allocator, &out, program.source_type).parse();
    assert!(reparsed.diagnostics.is_empty(), "output must parse: {out}");
    (out, program.source_type)
}

/// `[turbopack]_runtime.js` (minified Node.js development runtime): Node.js
/// loads it with `require`, so its top-level names are module locals, and the
/// on-demand loader declared inside `if (…) {}` is renamed with its Annex B `var`.
#[test]
fn node_runtime_script_is_commonjs() {
    let code = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function a(e){}c.l=a,c.w=1;let h=globalThis.__turbopack_server_hmr_handlers__;if(void 0!==globalThis.__turbopack_ensure_chunk__){function o(e){return a.call(this,e)}c.l=o}function m(p,i){}function r(p,d){}module.exports=p=>({m:i=>m(p,i),c:d=>r(p,d)});";
    let (out, source_type) = run_script(code);
    assert!(source_type.is_commonjs(), "{source_type:?}");
    for expected in [
        "function Context(e, t)",
        "function loadChunkAsync(e)",
        "function loadChunkAsyncOnDemand(e) {\n\t\treturn loadChunkAsync.call(this, e);",
        "contextPrototype.l = loadChunkAsyncOnDemand",
        "function getOrInstantiateRuntimeModule(p, i)",
        "(sourcePath) => ({",
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
}

#[test]
fn browser_runtime_script_keeps_its_globals() {
    let code = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function a(e){}c.l=a;";
    let (out, source_type) = run_script(code);
    assert!(source_type.is_script(), "{source_type:?}");
    assert!(out.contains("function g(e, t)"), "{out}");
    assert!(out.contains("function a(e)"), "{out}");
}

#[test]
fn node_chunks_and_entries_are_commonjs() {
    let (_, chunk) = run_script(r"module.exports=[1,e=>{e.v(1)}];");
    assert!(chunk.is_commonjs());
    let (_, entry) = run_script(
        r#"var R=require("./[turbopack]_runtime.js")("server/app/page.js");R.c("server/chunks/a.js");module.exports=R.m(1).exports;"#,
    );
    assert!(entry.is_commonjs());
}

/// Context members whose meaning changed between Next.js versions are named by
/// what is installed (16.0 `b = getWorkerBlobURL`, 16.2 `b = createWorker`,
/// 16.4 `b = RUNTIME_CHUNK_BASE_PATH`; `w = loadWebAssembly` in a browser).
#[test]
fn names_versioned_members_by_what_is_installed() {
    let old = run(
        r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function a(e){}function k(e){return new Blob([e])}function q(e){}c.l=a,c.b=k,c.w=q;",
    );
    for expected in [
        "function getWorkerBlobURL(e)",
        "function loadWebAssembly(e)",
        "function loadChunk(e)",
    ] {
        assert!(old.contains(expected), "{expected}: {old}");
    }
    let worker = run(r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;function k(e){return new Worker(e)}c.b=k;");
    assert!(worker.contains("function createWorker(e)"), "{worker}");
    let new = run(r#"function g(e,t){this.m=e,this.e=t}var c=g.prototype;let p="/_next/",x="?v=1";c.b=p,c.X=x;"#);
    assert!(new.contains("let RUNTIME_CHUNK_BASE_PATH = \"/_next/\""), "{new}");
    assert!(new.contains("ASSET_SUFFIX = \"?v=1\""), "{new}");
}

/// Calls the runtime source makes the same way in every version name their
/// callee, also from a member function the minifier inlined; the call graph
/// outranks the message anchors.
#[test]
fn names_runtime_functions_by_their_callers() {
    let code = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;
function I(e,t,r){return {id:e}}
let B=(e,t)=>I(e,1,t.id);
function w(e){let t=B(e,this.m);return t.exports}
function M(e,t,r){return fetch(r).catch(n=>{throw Error(`Failed to load chunk ${r} ${e}${t}`)})}
c.i=w;c.L=function(e){return M(1,this.m.id,e)};";
    let out = run(code);
    for expected in [
        "function esmImport(e)",
        "let getOrInstantiateModuleFromParent = (e, t) => instantiateModule(e, 1, t.id)",
        "function instantiateModule(e, t, r)",
        "function loadChunkByUrlInternal(e, t, r)",
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
    assert!(!out.contains("loadChunkByUrlWhole"), "{out}");
}

/// Next.js 16.0 throws the "factory is not available" message inline in
/// `instantiateModule`; only a function returning it is `factoryNotAvailableMessage`.
#[test]
fn message_builders_return_their_message() {
    let runtime = r"function g(e,t){this.m=e,this.e=t}var c=g.prototype;";
    let thrown = run(&format!(
        "{runtime}function A(e,t,r){{if(!r)throw Error(`Module ${{e}} was instantiated ${{t}}, but the module factory is not available.`)}}"
    ));
    assert!(thrown.contains("function A(e, t, r)"), "{thrown}");
    let returned = run(&format!(
        "{runtime}function A(e,t,r){{let n=t;return`Module ${{e}} was instantiated ${{n}}, but the module factory is not available.`}}"
    ));
    assert!(
        returned.contains("function factoryNotAvailableMessage(e, t, r)"),
        "{returned}"
    );
}
