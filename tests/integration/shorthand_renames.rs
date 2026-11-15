//! A rename must keep the property name of a shorthand property: `({ g } = o)`
//! becomes `({ g: v0 } = o)`, never `({ v0 } = o)`, which reads `o.v0`.
//! Every case runs under node before and after deobfuscation.

use std::path::Path;
use std::process::Command;

fn run_jsbeautify(dir: &Path, flags: &[&str], code: &str) -> String {
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

/// Each input, and what node prints for it.
const CASES: &[(&str, &str)] = &[
    ("(function(){ var g = 1; ({ g } = { g: 2 }); console.log(g) })()", "2"),
    (
        "(function(){ var g = 1, h; ({ g, h = 7 } = { g: 2 }); console.log(g, h) })()",
        "2 7",
    ),
    (
        "(function(){ var g, h; [{ g }, { x: { h } }] = [{ g: 3 }, { x: { h: 4 } }]; console.log(g, h); for ({ g } of [{ g: 9 }]) console.log(g) })()",
        "3 4\n9",
    ),
    (
        "(function(){ var g = 1, h = 2; console.log(JSON.stringify({ g, h })) })()",
        r#"{"g":1,"h":2}"#,
    ),
    (
        "(function(){ var { g, h = 5 } = { g: 1 }; function f({ a, b = 2 }) { return a + b } var k = ({ q }) => q; try { throw { e: 8 } } catch ({ e }) { console.log(e) } console.log(g, h, f({ a: 1 }), k({ q: 3 })) })()",
        "8\n1 5 3 3",
    ),
    (
        "var g = 1, h = { g: 0 }; ({ g } = { g: 2 }); console.log(g, JSON.stringify({ g }), JSON.stringify(h))",
        r#"2 {"g":2} {"g":0}"#,
    ),
    (
        "var webpackJsonp = function(c, m) { for (var k in m) m[k]({}, {}, null) };\nwebpackJsonp([0], { 1: function(__proto__, t, n) { t.o = { __proto__ }; console.log(Object.keys(t.o).join(), Object.getPrototypeOf(t.o) === Object.prototype) } });",
        "__proto__ true",
    ),
];

#[test]
fn renamed_shorthand_properties_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-shorthand-renames-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["--rename-vars"], &["-d", "--isolated"]] {
        for &(input, expected) in CASES {
            let output = run_jsbeautify(&dir, flags, input);
            let Some(input_stdout) = node_stdout(&dir, input) else {
                eprintln!("node not found; skipping the behavior comparison");
                let _ = std::fs::remove_dir_all(&dir);
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
