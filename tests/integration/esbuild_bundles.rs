//! esbuild / Bun runtime helpers: export-name recovery and bundler labels.

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
    assert!(out.contains("//   __esm → __esm"), "{out}");
    assert!(!out.contains("Runtime:"), "{out}");
}
