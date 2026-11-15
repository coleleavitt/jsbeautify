//! Proxy and forwarder passes inline calls of bindings found by
//! `call_sites::plain_call_arities`. A direct `eval` can call, reassign or
//! shadow such a binding by name, so a binding it can see keeps its calls
//! and its declaration.

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
/// `var bmak, _abck` make the detector run the Akamai forwarder passes.
const CASES: &[(&str, &str)] = &[
    // operator proxy reassigned by eval
    (
        r#"(function () { function BI(a, b) { return a === b; } eval("BI = function (a, b) { return a + b; }"); console.log(BI(1, 2), BI(3, 4)); })();"#,
        "3 7",
    ),
    // operator proxy shadowed by an eval `var` in a nested function
    (
        r#"(function () { var BI = function (a, b) { return a * b; }; function g() { eval("var BI = function (a, b) { return a - b; }"); return BI(7, 2); } console.log(g(), BI(3, 4)); })();"#,
        "5 12",
    ),
    // operator proxy called by name inside eval
    (
        r#"(function () { function BI(a, b) { return a + b; } console.log(BI(1, 2), BI(3, 4), eval("BI(5, 6)")); })();"#,
        "3 7 11",
    ),
    // unary proxy reassigned by eval
    (
        r#"(function () { function Bd(x) { return !x; } eval("Bd = function (x) { return 'e' + x; }"); console.log(Bd(1), Bd(0)); })();"#,
        "e1 e0",
    ),
    // lookup forwarder reassigned by eval (Akamai phase 0.5e)
    (
        r#"var bmak = 1, _abck = 2; (function () { function A() { return ["a", "b", "c"]; } function F(i) { return A()[i]; } eval("F = function (i) { return 'e' + i; }"); console.log(F(0), F(2)); })();"#,
        "e0 e2",
    ),
    // method-call forwarder reassigned by eval (Akamai phase 0.5e2)
    (
        r#"var bmak = 1, _abck = 2; (function () { function M(o, a) { return o.concat(a); } eval("M = function (o, a) { return 'e'; }"); console.log(M([1], 2), M([3], 4)); })();"#,
        "e e",
    ),
];

#[test]
fn proxies_visible_to_direct_eval_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-direct-eval-{}", std::process::id()));
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
