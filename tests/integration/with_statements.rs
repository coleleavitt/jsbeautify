//! `with` statements: inside `with (obj) { … }` a name first reads `obj`'s
//! property of that name. Renaming a binding a `with` body reads, inlining
//! it there, or writing a global name such as `undefined` there changes
//! which of the two the program reads, so deobfuscation must keep them.

use std::path::Path;
use std::process::Command;

fn deobfuscate(dir: &Path, code: &str, flags: &[&str]) -> String {
    let path = dir.join("input.js");
    std::fs::write(&path, code).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(flags)
        .arg(&path)
        .output()
        .expect("run jsbeautify");
    assert!(output.status.success(), "jsbeautify {flags:?} failed: {output:?}");
    String::from_utf8(output.stdout).expect("stdout is utf8")
}

/// What `code` prints under node, or `None` when node is not installed.
fn node_stdout(dir: &Path, code: &str) -> Option<String> {
    let path = dir.join("run.js");
    std::fs::write(&path, code).expect("write script");
    let output = match Command::new("node").arg(&path).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("run node: {error}"),
    };
    assert!(output.status.success(), "node failed: {output:?}");
    Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
}

/// Each input prints what node prints for it, and its deobfuscated output
/// prints the same.
const CASES: &[(&str, &str)] = &[
    (
        r"var o={D:function(){return 'o'}}; function D(){return 'outer'} with(o){console.log(D())}",
        "o",
    ),
    (
        r"(function(){var o={D:function(){return 'o'}}; function D(){return 'outer'+Math.random().toString().slice(0,0)} with(o){console.log(D())}})();",
        "o",
    ),
    (
        r"(function(){function hello(H){ var o={H:'o'}; with(o){ console.log(H) } } hello('outer'); function world(a){var o={a:'o'}; with(o){console.log(a)}} world('outer')})();",
        "o\no",
    ),
    (
        r"(function(){var o={k:1}; with(o){ var k = 2; } console.log(k, o.k); for (var _0xa in {z:1}) {} with(o){ var _0xq = 5 } console.log(_0xq, o._0xq)})();",
        "undefined 2\n5 undefined",
    ),
    (
        r"function f(){ var o={a:function(){return 'o'}}; function a(){return 'outer'} with(o){console.log(a())} } f();",
        "o",
    ),
    (
        r"(function(){var o={_0x2b:function(a,b){return a*b}}; function _0x2b(a,b){return a+b} with(o){console.log(_0x2b(3,4)); console.log(_0x2b(5,6));} console.log(_0x2b(1,2));})();",
        "12\n30\n3",
    ),
    (
        r"(function(){var o={_0xu:function(a){return 'o'}}; function _0xu(a){return !a} with(o){console.log(_0xu(0)); console.log(_0xu(1));} console.log(_0xu(0));})();",
        "o\no\ntrue",
    ),
    (
        r#"(function(){var _0x1234=["hello","world"]; function _0xdec(a){return _0x1234[a]} var o={_0xdec:function(){return 'o'}}; with(o){console.log(_0xdec(0))} console.log(_0xdec(1))})();"#,
        "o\nworld",
    ),
    (
        r#"(function(){var d={"a":function(){return 'outer'}}; var o={d:{"a":function(){return 'o'}}}; with(o){console.log(d["a"]())} console.log(d["a"]())})();"#,
        "o\nouter",
    ),
    (
        r#"(function(){var I=2; function f(){return ["a","b","c"]} var o={f:function(){return ['o','o','o']}, I:0}; with(o){console.log(f()[I])} console.log(f()[I])})();"#,
        "o\nc",
    ),
    (
        r"globalThis.window=globalThis; globalThis.setTimeout2=function(){return 'g'}; (function(){ var w = window; var o={setTimeout2:function(){return 'o'}, Math:{max:function(){return 'omax'}}}; with(o){ console.log(w.setTimeout2(), w.Math.max(1,2)) } console.log(w.Math.max(1,2)) })();",
        "g 2\n2",
    ),
    (
        r"(function(){var o={undefined:'ou'}; var x; with(o){ x = void 0; console.log(x) }})();",
        "undefined",
    ),
    (
        r"(function(){var o={String:{fromCharCode:function(){return 'o'}}}; with(o){ console.log(String.fromCharCode(104,105)) }})();",
        "o",
    ),
    (
        r"(function(){var o={NaN:'oNaN', Infinity:'oInf'}; with(o){ console.log(0/0, 1/0, -1/0) }})();",
        "NaN Infinity -Infinity",
    ),
    (
        r"(function(){var o={}; with(o){ v0 = 5 } console.log(typeof o, globalThis.v0)})();",
        "object 5",
    ),
    (
        r"(function(){var value='outer'; function hello(H, q){ return [value, H, q].join() } console.log(hello('a','b'))})();",
        "outer,a,b",
    ),
];

#[test]
fn deobfuscated_with_bodies_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-with-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["-d", "--isolated"], &["-d", "--rename-vars"]] {
        for &(input, expected) in CASES {
            let output = deobfuscate(&dir, input, flags);
            let Some(input_stdout) = node_stdout(&dir, input) else {
                eprintln!("node not found; skipping the behavior comparison");
                return;
            };
            assert_eq!(input_stdout.trim_end(), expected, "{input}");
            assert_eq!(
                node_stdout(&dir, &output).as_deref().map(str::trim_end),
                Some(expected),
                "{flags:?} {input}\n=>\n{output}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
