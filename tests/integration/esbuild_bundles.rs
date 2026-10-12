//! esbuild / Bun runtime helpers and native ES module exports: export-name
//! recovery and bundler labels.

use js_beautify_rs::{AstDeobfuscator, ParseGoal};

const ESBUILD_RUNTIME: &str = "var d=Object.defineProperty;\
    var X=(e,t)=>{for(var n in t)d(e,n,{get:t[n],enumerable:!0})};\
    var E=(e,t)=>()=>(e&&(t=e(e=0)),t);\n";

fn deobfuscate(code: &str) -> String {
    AstDeobfuscator::new()
        .with_parse_goal(ParseGoal::CommonJs)
        .deobfuscate(code)
        .expect("deobfuscate")
}

fn esbuild_bundle(tail: &str) -> String {
    format!(
        "{ESBUILD_RUNTIME}var a={{}};X(a,{{startServer:()=>s,stopServer:()=>p}});\
         function s(){{return 1}}function p(){{return 2}}var m=E(()=>{{}});{tail}\
         module.exports={{a,m}};"
    )
}

#[test]
fn recovers_export_names_next_to_an_identifier_eval() {
    let out = deobfuscate(&esbuild_bundle(
        "function load(){return eval(\"require\")(\"stream\")}load();",
    ));
    assert!(out.contains("function startServer()"), "{out}");
    assert!(out.contains("function stopServer()"), "{out}");
    assert!(out.contains("eval(\"require\")"), "{out}");
    assert!(out.contains("// Runtime: esbuild"), "{out}");
    assert!(out.contains("// esbuild ESM module: init_1"), "{out}");
}

#[test]
fn never_renames_a_binding_to_the_name_an_eval_reads() {
    let out = deobfuscate(&esbuild_bundle("function load(){return eval(\"startServer\")}load();"));
    assert!(out.contains("function startServer$1()"), "{out}");
    assert!(!out.contains("function startServer()"), "{out}");
    assert!(out.contains("eval(\"startServer\")"), "{out}");
}

#[test]
fn guards_the_binding_an_identifier_eval_resolves_to() {
    let out = deobfuscate(&esbuild_bundle("function load(){return eval(\"s\")}load();"));
    assert!(out.contains("function s()"), "{out}");
    assert!(out.contains("function stopServer()"), "{out}");
}

#[test]
fn renames_nothing_visible_to_a_dynamic_eval() {
    let out = deobfuscate(&esbuild_bundle("function load(c){return eval(c)}load(\"s\");"));
    assert!(out.contains("function s()") && out.contains("function p()"), "{out}");
}

#[test]
fn labels_a_bun_bundle_from_its_export_helper() {
    let code = "var d=Object.defineProperty;\
        var X=(e,t)=>{for(var n in t)d(e,n,{get:t[n],enumerable:!0,configurable:!0,set:(r)=>t[n]=()=>r})};\
        var a={};X(a,{startServer:()=>s});function s(){return 1}module.exports=a;";
    let out = deobfuscate(code);
    assert!(out.contains("// Runtime: Bun"), "{out}");
    assert!(out.contains("function startServer()"), "{out}");
}

#[test]
fn names_exports_listed_in_the_cjs_export_names_hint() {
    // esbuild appends `0 && (module.exports = {...})` for Node's
    // cjs-module-lexer; its free references are never evaluated.
    let out = deobfuscate(
        "var d=Object.defineProperty;\
         var X=(e,t)=>{for(var n in t)d(e,n,{get:t[n],enumerable:!0})};\
         var a={};X(a,{AgentSkill:()=>yr});var yr=class{};module.exports=a;\
         0&&(module.exports={AgentSkill});",
    );
    assert!(out.contains("var AgentSkill = class"), "{out}");
    assert!(!out.contains("AgentSkill$1"), "{out}");
}

#[test]
fn keeps_avoiding_a_free_global_read_outside_the_hint() {
    let out = deobfuscate(
        "var d=Object.defineProperty;\
         var X=(e,t)=>{for(var n in t)d(e,n,{get:t[n],enumerable:!0})};\
         var a={};X(a,{AgentSkill:()=>yr});var yr=class{};module.exports=a;\
         AgentSkill.load();0&&(module.exports={AgentSkill});",
    );
    assert!(out.contains("var AgentSkill$1 = class"), "{out}");
    assert!(out.contains("AgentSkill.load()"), "{out}");
}

#[test]
fn omits_the_bundler_without_a_distinguishing_helper() {
    let out = deobfuscate("var E=(e,t)=>()=>(e&&(t=e(e=0)),t);var m=E(()=>{});module.exports=m;");
    assert!(out.contains("//   __esm\n"), "{out}");
    assert!(!out.contains("Runtime:"), "{out}");
}

#[test]
fn names_default_export_locals_after_the_module() {
    // Shapes from `esbuild --bundle --format=cjs --minify` and `--minify-identifiers`.
    let out = deobfuscate(
        "var d=Object.defineProperty;\
         var X=(e,t)=>{for(var n in t)d(e,n,{get:t[n],enumerable:!0})};\
         var a={};X(a,{default:()=>i,size:()=>h});function i(t){return{label:t}}var h=3;\
         var p={};X(p,{default:()=>c});var c=class extends Error{constructor(){super(\"x\"),this.name=\"CodecError\"}};\n\
         // src/anon.ts\n\
         var x={};X(x,{default:()=>y});var y={a:1};module.exports={a,p,x};",
    );
    assert!(
        out.contains("function default_1(") && out.contains("default: () => default_1"),
        "{out}"
    );
    assert!(out.contains("var size = 3"), "{out}");
    assert!(out.contains("var CodecError = class extends Error"), "{out}");
    assert!(out.contains("var anon_default = { a: 1 }"), "{out}");
}

#[test]
fn names_the_lazy_modules_of_an_esbuild_0_28_bundle() {
    // `esbuild 0.28.2 --bundle --format=cjs --minify --platform=node`: `__esm`
    // and `__commonJS` wrap the module call in `try`.
    let out = deobfuscate(
        r#"var c=Object.defineProperty;var x=Object.getOwnPropertyDescriptor;var q=Object.getOwnPropertyNames;var b=Object.prototype.hasOwnProperty;var l=(r,e,t)=>()=>{if(t)throw t[0];try{return r&&(e=r(r=0)),e}catch(s){throw t=[s],s}};var d=(r,e)=>()=>{try{return e||r((e={exports:{}}).exports,e),e.exports}catch(t){throw e=0,t}},n=(r,e)=>{for(var t in e)c(r,t,{get:e[t],enumerable:!0})},f=(r,e,t,s)=>{if(e&&typeof e=="object"||typeof e=="function")for(let o of q(e))!b.call(r,o)&&o!==t&&c(r,o,{get:()=>e[o],enumerable:!(s=x(e,o))||s.enumerable});return r};var i=r=>f(c({},"__esModule",{value:!0}),r);var w={};n(w,{answer:()=>g});var g,a=l(()=>{g=42});var u=d((y,p)=>{var h=(a(),i(w));p.exports={twice:()=>h.answer*2}});var m={};n(m,{result:()=>k});module.exports=i(m);var j=u(),k=j.twice();0&&(module.exports={result});"#,
    );
    for helper in ["__esm", "__commonJS", "__export", "__copyProps", "__toCommonJS"] {
        assert!(out.contains(&format!("var {helper} = ")), "{helper}: {out}");
        assert!(out.contains(&format!("//   {helper}\n")), "{helper}: {out}");
    }
    assert!(out.contains("var init_1 = __esm("), "{out}");
    assert!(out.contains("var require_1 = __commonJS("), "{out}");
    assert!(out.contains("// esbuild ESM module: init_1"), "{out}");
    assert!(out.contains("// esbuild CommonJS module: require_1"), "{out}");
}

fn deobfuscate_module(code: &str) -> String {
    AstDeobfuscator::new()
        .with_parse_goal(ParseGoal::Module)
        .deobfuscate(code)
        .expect("deobfuscate")
}

#[test]
fn names_minified_locals_after_their_native_export_names() {
    // The export tail of a minified vite/rollup chunk.
    let out = deobfuscate_module(
        "function a(){return 1}var b=class{};const c=()=>a();let Button=0;\
         export{a as useThing,b as Store,c as Button,Button as Old};",
    );
    assert!(out.contains("function useThing()"), "{out}");
    assert!(out.contains("var Store = class"), "{out}");
    assert!(out.contains("const Button$1 = () => useThing()"), "{out}");
    assert!(
        out.contains("useThing, Store, Button$1 as Button, Button as Old"),
        "public names must not change: {out}"
    );
}

#[test]
fn keeps_locals_whose_export_name_says_nothing_or_cannot_be_bound() {
    let out = deobfuscate_module(
        "var r=1,s=2,t=3,u=4;export{r as a,s as \"foo-bar\",t as delete,u as default};export const v=5;",
    );
    assert!(out.contains("r as a"), "{out}");
    assert!(out.contains("s as \"foo-bar\""), "{out}");
    assert!(out.contains("t as delete"), "{out}");
    assert!(out.contains("export const v = 5"), "{out}");
}

#[test]
fn keeps_native_export_locals_an_eval_can_read() {
    let out = deobfuscate_module("var a=1;function f(c){return eval(c)}export{a as Count,f as run};");
    assert!(out.contains("a as Count"), "{out}");
}

#[test]
fn names_the_runtime_helpers_a_split_chunk_exports() {
    // `esbuild --splitting --format=esm --minify`: the shared chunk exports its helpers.
    let out = deobfuscate_module("var s=(t,r)=>()=>(t&&(r=t(t=0)),r);export{s as a};");
    assert!(out.contains("var __esm = "), "{out}");
    assert!(out.contains("export { __esm as a }"), "{out}");
}
