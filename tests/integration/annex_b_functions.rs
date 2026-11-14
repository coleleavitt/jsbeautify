//! Sloppy-mode block functions: Annex B copies a function declared in a block
//! into a same-named `var` of the enclosing function when the block runs, so a
//! call `oxc_semantic` resolves to an outer declaration may run the block
//! function instead. Deobfuscation must keep such calls.

use std::path::Path;
use std::process::Command;

fn deobfuscate(dir: &Path, code: &str) -> String {
    let path = dir.join("input.js");
    std::fs::write(&path, code).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("-d")
        .arg(&path)
        .output()
        .expect("run jsbeautify -d");
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
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
        "function g(){ function f(){return 1} { function f(){return 2} } return f(); } console.log(g())",
        "2",
    ),
    (
        "function g(){ function f(){return 1} if (true) { function f(){return 2} } return f(); } console.log(g())",
        "2",
    ),
    (
        "function g(){ function f(){return 1} if (false) function f(){return 2} return f(); } console.log(g())",
        "1",
    ),
    (
        "function g(x){ function f(){return 1} switch (x) { case 1: function f(){return 2} } return f(); } console.log(g(1), g(0))",
        "2 1",
    ),
    (
        "function g(){ function f(){return 1} l: { function f(){return 2} } return f(); } console.log(g())",
        "2",
    ),
    (
        "function g(){ function f(){return 1} try { throw 0 } catch (f) { { function f(){return 2} } } return f(); } console.log(g())",
        "2",
    ),
    (
        "function g(){ function f(){return 1} { let f = 3; { function f(){return 2} } } return f(); } console.log(g())",
        "1",
    ),
    (
        "function t(a){return a+1} function g(){ function p(a){return t(a)} { function p(a){return 7} } return p(1); } console.log(g())",
        "7",
    ),
    (
        "function g(){ function o(a,b){return a+b} { function o(a,b){return a*b} } return o(3,4); } console.log(g())",
        "12",
    ),
];

#[test]
fn calls_an_annex_b_block_function_may_replace_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-annex-b-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for &(input, expected) in CASES {
        let output = deobfuscate(&dir, input);
        let Some(input_stdout) = node_stdout(&dir, input) else {
            eprintln!("node not found; skipping the behavior comparison");
            break;
        };
        assert_eq!(input_stdout.trim_end(), expected, "{input}");
        assert_eq!(
            node_stdout(&dir, &output).as_deref().map(str::trim_end),
            Some(expected),
            "{input}\n=>\n{output}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
