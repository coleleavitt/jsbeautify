//! javascript-obfuscator string-array fixtures: every decoder call is
//! decoded, and the output prints what the input prints under node.

use std::io::Write;
use std::process::{Command, Stdio};

use jsbeautify::AstDeobfuscator;
use regex::Regex;

use crate::fixtures;

/// The stdout of `code` run by node, or `None` when node is not installed.
fn node_stdout(code: &str) -> Option<String> {
    let mut child = Command::new("node")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()
        .expect("node stdin")
        .write_all(code.as_bytes())
        .expect("write to node");
    let output = child.wait_with_output().expect("run node");
    assert!(output.status.success(), "node failed on:\n{code}");
    Some(String::from_utf8(output.stdout).expect("utf-8 stdout"))
}

fn deobfuscate(name: &str, src: &str) -> String {
    AstDeobfuscator::new()
        .deobfuscate(src)
        .unwrap_or_else(|e| panic!("deobfuscate {name}: {e}"))
}

fn assert_same_stdout(name: &str, src: &str, out: &str) {
    match (node_stdout(src), node_stdout(out)) {
        (Some(expected), Some(actual)) => assert_eq!(actual, expected, "{name} behaves differently:\n{out}"),
        _ => eprintln!("node is not installed; skipped the behaviour check for {name}"),
    }
}

fn assert_decoded(name: &str, decoder: &str) {
    let src = fixtures::obfuscator(name);
    let out = deobfuscate(name, &src);

    let numeric_call = Regex::new(r"\b[\w$]+\((?:0x[0-9a-fA-F]+|\d+)\)").expect("regex");
    assert!(!numeric_call.is_match(&out), "a decoder call is left in {name}:\n{out}");
    assert!(!out.contains("parseInt"), "the rotation IIFE is left in {name}:\n{out}");
    assert!(
        out.matches(&format!("{decoder}(")).count() <= 1,
        "only the decoder's declaration may name it in {name}:\n{out}"
    );
    assert_same_stdout(name, &src, &out);
}

#[test]
fn decodes_default_obfuscator_string_array() {
    assert_decoded("string_array_default.js", "_0x4079");
}

#[test]
fn decodes_string_array_through_nested_aliases() {
    assert_decoded("string_array_nested_aliases.js", "_0x4285");
}

#[test]
fn rotates_a_countdown_string_array_like_node() {
    let src = fixtures::obfuscated("simple_string_array.js");
    let out = deobfuscate("simple_string_array.js", &src);
    assert!(out.matches("_0xdec(").count() <= 1, "{out}");
    assert_same_stdout("simple_string_array.js", &src, &out);
}

#[test]
fn unflattens_a_dispatch_sequence_read_from_the_string_array() {
    let src = "
        function _0xarr() { const a = ['log', '1|0|2', 'split', 'first', 'second', 'third']; _0xarr = function () { return a; }; return _0xarr(); }
        function _0xdec(i, k) { i = i - 0x100; const s = _0xarr(); let v = s[i]; return v; }
        function run() {
            const d = _0xdec;
            const order = d(0x101)[d(0x102)]('|');
            let i = 0;
            while (true) {
                switch (order[i++]) {
                    case '0': console[d(0x100)](d(0x103)); continue;
                    case '1': console[d(0x100)](d(0x104)); continue;
                    case '2': console[d(0x100)](d(0x105)); continue;
                }
                break;
            }
        }
        run();
    ";
    let out = deobfuscate("dispatch sequence", src);
    assert!(!out.contains("while") && !out.contains(".split("), "{out}");
    assert_same_stdout("dispatch sequence", src, &out);
}
