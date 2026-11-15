//! Rewrites that move or merge code keep its order observable: a value
//! read earlier, a binding resolved by name, or a cache hoisted to another
//! scope must not change what the program prints.

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

/// Each input and what node prints for it and for its deobfuscated output.
/// `var bmak, _abck` make the detector run the Akamai passes.
const CASES: &[(&str, &str)] = &[
    // object sparsing: the object reads itself before its literal is assigned
    (
        r"(function () { var o = {}; o.self = o; o.n = 1; console.log(o.self === o, o.n); })();",
        "true 1",
    ),
    // object sparsing: a `with` setter sees the empty object first
    (
        r#"(function () { var w = { get o() { return this._o; }, set o(v) { this._o = v; console.log("set", JSON.stringify(v)); } }; with (w) { var o = {}; o.a = 1; } console.log(JSON.stringify(w._o)); })();"#,
        "set {}\n{\"a\":1}",
    ),
    // object sparsing: a global getter runs after the earlier properties are set
    (
        r#"Object.defineProperty(globalThis, "g", { get() { console.log("get", typeof o, o && o.a); return 2; } }); var o = {}; o.a = 1; o.b = g; console.log(JSON.stringify(o));"#,
        "get object 1\n{\"a\":1,\"b\":2}",
    ),
    // object sparsing: an undeclared name throws after the earlier properties are set
    (
        r"(function () { try { var o = {}; o.a = 1; o.b = missing; } catch (e) { console.log(e.name); } console.log(JSON.stringify(o)); })();",
        "ReferenceError\n{\"a\":1}",
    ),
    // object sparsing: a `let` in its TDZ throws after the earlier properties are set
    (
        r"(function () { try { var o = {}; o.a = 1; o.b = later; let later = 2; } catch (e) { console.log(e.name); } console.log(JSON.stringify(o)); })();",
        "ReferenceError\n{\"a\":1}",
    ),
    // object sparsing: literals, vars, functions and `undefined` still move
    (
        r#"(function () { var x = 5; function f() { return 1; } var o = {}; o.a = 1; o.b = "s"; o.c = x; o.d = f; o.e = undefined; o.f = null; console.log(JSON.stringify(o), o.d()); })();"#,
        "{\"a\":1,\"b\":\"s\",\"c\":5,\"f\":null} 1",
    ),
    // self-init accessor: each run of the outer function gets its own cache
    (
        r"var bmak = 1, _abck = 2; (function () { function outer() { function F() { var x = []; F = function () { return x; }; return x; } return F(); } console.log(outer() === outer()); })();",
        "false",
    ),
    // self-init accessor: the cache name does not clobber an existing `__F_cache`
    (
        r#"var bmak = 1, _abck = 2; var __F_cache = "mine"; (function () { function F() { var x = []; F = function () { return x; }; return x; } F().push(1); console.log(F().length, __F_cache); })();"#,
        "1 mine",
    ),
    // self-init accessor: a local `Object` is not the global
    (
        r"var bmak = 1, _abck = 2; (function () { var Object = function () { this.tag = 1; }; function F() { var x = new Object(); F = function () { return x; }; return x; } console.log(F().tag); })();",
        "1",
    ),
    // self-init accessor: `Object.create(proto)` keeps its prototype
    (
        r"var bmak = 1, _abck = 2; (function () { var proto = { hi: 1 }; function F() { var x = Object.create(proto); F = function () { return x; }; return x; } console.log(F().hi); })();",
        "1",
    ),
    // self-init accessor: a captured original function initialises on every call
    (
        r"var bmak = 1, _abck = 2; (function () { function F() { var x = []; F = function () { return x; }; return x; } var g = F; console.log(g() === g(), F() === F()); })();",
        "false true",
    ),
    // self-init accessor: an async accessor's later calls return the object itself
    (
        r"var bmak = 1, _abck = 2; (function () { async function F() { var x = []; F = function () { return x; }; return x; } F(); console.log(F() instanceof Promise); })();",
        "false",
    ),
    // self-init accessor: `[]` and `Object.create(null)` accessors still flatten
    (
        r"var bmak = 1, _abck = 2; (function () { function F() { var x = []; F = function () { return x; }; return x; } function G() { var y = Object.create(null); G = function () { return y; }; return y; } F().push(1); console.log(F().length, F() === F(), Object.getPrototypeOf(G()), G() === G()); })();",
        "1 true null true",
    ),
];

#[test]
fn reordering_rewrites_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-rewrite-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["-d", "--isolated"]] {
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
