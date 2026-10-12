//! `jsbeautify -d --project <dir> -o <outdir>`: cross-file naming of a code-split build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// esbuild `--splitting --minify` output: the runtime chunk exports its helpers
/// under one-letter names that the other chunks import.
const RUNTIME: &str = r#"var l=Object.create;var i=Object.defineProperty;var h=Object.getOwnPropertyDescriptor;var g=Object.getOwnPropertyNames;var E=Object.getPrototypeOf,v=Object.prototype.hasOwnProperty;var w=(r,t)=>()=>{try{return t||r((t={exports:{}}).exports,t),t.exports}catch(e){throw t=0,e}},b=(r,t)=>{for(var e in t)i(r,e,{get:t[e],enumerable:!0})},z=(r,t,e,p)=>{if(t&&typeof t=="object"||typeof t=="function")for(let o of g(t))!v.call(r,o)&&o!==e&&i(r,o,{get:()=>t[o],enumerable:!(p=h(t,o))||p.enumerable});return r};var M=(r,t,e)=>(e=r!=null?l(E(r)):{},z(t||!r||!r.__esModule?i(e,"default",{value:r,enumerable:!0}):e,r));var o=class extends Error{constructor(t){super(t),this.name="ValidationError"}};var n={n:0};export{w as a,b,M as c,o as d,n as h};"#;
const UTIL: &str = r#"import{b as r}from"./chunk-SNUETOXO.js";var i={};r(i,{capitalize:()=>p,repeatTimes:()=>n});function p(e){return e[0].toUpperCase()+e.slice(1)}function n(e,t){return e.repeat(t)}export{p as a,n as b,i as c};"#;
const ENTRY: &str = r#"import{b as i,c as u}from"./chunk-JCZOX377.js";import{a as c,c as m,d as t,h as e}from"./chunk-SNUETOXO.js";var s=c((h,o)=>{o.exports=function(r){return"hi "+r}});var p=m(s());function l(){return[(0,p.default)("b"),e.n,new t("q").name,i("ab",2)]}var y=r=>"h"+r;export{y as h,l as runB,u as util};"#;

fn project(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("jsb-project-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let src = dir.join("dist");
    fs::create_dir_all(src.join("assets")).expect("create project dir");
    fs::write(src.join("assets/chunk-SNUETOXO.js"), RUNTIME).expect("write chunk");
    fs::write(src.join("assets/chunk-JCZOX377.js"), UTIL).expect("write chunk");
    fs::write(src.join("assets/entry.js"), ENTRY).expect("write entry");
    fs::write(
        src.join("index.html"),
        "<script type=module src=assets/entry.js></script>",
    )
    .expect("write html");
    (dir, src)
}

fn run(src: &Path, out: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(["-d", "--project"])
        .arg(src)
        .arg("-o")
        .arg(out)
        .output()
        .expect("run jsbeautify --project")
}

#[test]
fn project_names_bindings_across_chunks() {
    let (dir, src) = project("names");
    let out = dir.join("out");
    let output = run(&src, &out);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("3 files deobfuscated, 3 passed both re-parse gates"),
        "{stderr}"
    );
    assert!(stderr.contains("0 broken"), "{stderr}");
    assert_eq!(
        fs::read_to_string(out.join("index.html")).expect("other files are copied"),
        "<script type=module src=assets/entry.js></script>"
    );

    let util = fs::read_to_string(out.join("assets/chunk-JCZOX377.js")).expect("util output");
    assert!(util.contains("b as __export"), "{util}");
    assert!(util.contains("function capitalize("), "{util}");
    assert!(
        util.contains("var util = {}"),
        "named after entry.js's public re-export: {util}"
    );
    assert!(util.contains("capitalize as a, repeatTimes as b, util as c"), "{util}");

    let entry = fs::read_to_string(out.join("assets/entry.js")).expect("entry output");
    for expected in [
        "b as repeatTimes, c as util",
        "a as __commonJS, c as __toESM, d as ValidationError, h as chunkSNUETOXO_h",
        "var require_1 = __commonJS(",
        "chunkSNUETOXO_h.n",
        "export {\n    h, runB, util\n}",
    ] {
        assert!(entry.contains(expected), "missing {expected:?} in {entry}");
    }
    let runtime = fs::read_to_string(out.join("assets/chunk-SNUETOXO.js")).expect("runtime output");
    assert!(runtime.contains("var chunkSNUETOXO_h = {"), "{runtime}");

    let index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("project-index.json")).expect("index written"))
            .expect("index is JSON");
    assert_eq!(index["stats"]["imported_helpers"], 3);
    assert_eq!(index["links"]["inconsistent_names"], 0);
    assert!(out.join(".jsbeautify/logs/assets/entry.js.log").is_file());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn project_output_must_be_another_directory() {
    let (dir, src) = project("same-dir");
    let output = run(&src, &src);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not be the project directory"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn single_file_output_ignores_project_naming() {
    let (dir, src) = project("single");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("-d")
        .arg(src.join("assets/entry.js"))
        .output()
        .expect("run jsbeautify -d");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        !stdout.contains("__commonJS"),
        "a single chunk cannot know its imports: {stdout}"
    );
    assert!(stdout.contains(" as util"), "{stdout}");
    let _ = fs::remove_dir_all(&dir);
}
