//! Input nested far deeper than the main thread's stack allows: every entry
//! point sizes its worker stack from the input length, so none of these may
//! overflow.

use jsbeautify::ParseGoal;
use jsbeautify::inspect::scope::symbol_table;
use std::io::Write;
use std::process::{Command, Stdio};

fn shapes() -> Vec<(&'static str, String)> {
    let chain = |head: &str, link: &str, n: usize| format!("x = {head}{};\n", link.repeat(n));
    vec![
        (
            "brackets",
            format!("x = {}{};\n", "[".repeat(10_000), "]".repeat(10_000)),
        ),
        (
            "parens",
            format!("x = {}0{};\n", "(".repeat(10_000), ")".repeat(10_000)),
        ),
        ("calls", format!("{}{};\n", "f(".repeat(10_000), ")".repeat(10_000))),
        ("plus", chain("a", "+a", 40_000)),
        ("string concat", chain("'a'", "+'a'", 40_000)),
        ("and", chain("a", "&&a", 40_000)),
        ("member", chain("a", ".a", 40_000)),
        ("unary", format!("x = {}y;\n", "!".repeat(40_000))),
        ("ternary", format!("x = {}c;\n", "a ? b : ".repeat(20_000))),
        ("arrows", format!("x = {}0;\n", "() => ".repeat(20_000))),
        (
            "else if",
            format!("if (a) b();{}\n", " else if (a) b();".repeat(20_000)),
        ),
    ]
}

fn jsbeautify(args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn jsbeautify");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for jsbeautify");
    assert!(
        output.status.success(),
        "{args:?} failed: {:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

fn assert_reparses(shape: &str, code: &str) {
    let inspection = symbol_table(code, ParseGoal::Auto, false).expect("output parses");
    assert!(
        inspection.diagnostics.is_empty(),
        "{shape}: output does not re-parse cleanly: {:?}",
        inspection.diagnostics
    );
}

#[test]
fn deep_input_beautifies() {
    for (shape, source) in shapes() {
        assert_reparses(shape, &jsbeautify(&["-"], &source));
    }
}

#[test]
fn deep_input_deobfuscates() {
    for (shape, source) in shapes() {
        assert_reparses(shape, &jsbeautify(&["-d", "-"], &source));
    }
}

#[test]
fn deep_input_has_a_symbol_table() {
    for (shape, source) in shapes() {
        let report: serde_json::Value =
            serde_json::from_str(&jsbeautify(&["scope", "-"], &source)).expect("scope prints JSON");
        assert!(report["unresolved"].is_array(), "{shape}: {report}");
    }
}
