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

/// Shapes the codegen indents once per nesting level, `depth` levels deep.
/// Each prints what node prints for it.
fn indented_shapes(depth: usize) -> Vec<(&'static str, String, &'static str)> {
    let nest = |open: &str, middle: &str, close: &str| format!("{}{middle}{}", open.repeat(depth), close.repeat(depth));
    vec![
        ("blocks", nest("{", "console.log(1);", "}"), "1"),
        (
            "functions",
            format!(
                "var r = {};\nconsole.log(r);\n",
                nest("(function () { return ", "1", "})()")
            ),
            "1",
        ),
        (
            "objects",
            format!("var o = {};\nconsole.log(typeof o.a);\n", nest("{a: ", "1", "}")),
            "object",
        ),
        (
            "nested ifs",
            format!("var x = 1;\n{}", nest("if (x) ", "console.log(x);", "")),
            "1",
        ),
        (
            "switches",
            format!("var x = 1;\n{}", nest("switch (x) { case 1: ", "console.log(x);", "}")),
            "1",
        ),
        ("do-while", nest("do ", "console.log(1);", " while (0);"), "1"),
        (
            "classes",
            format!(
                "var c = {};\nconsole.log(c);\n",
                nest("new class { m() { return ", "1", "} }().m()")
            ),
            "1",
        ),
        (
            "arrow sequences",
            format!(
                "var g = () => 0;\nvar r = {};\nwhile (typeof r === 'function') r = r(g);\nconsole.log(r);\n",
                nest("(a) => (a(), a(), ", "1", ")")
            ),
            "1",
        ),
    ]
}

/// What `code` prints under node, or `None` when node is not installed.
fn node_stdout(dir: &std::path::Path, code: &str) -> Option<String> {
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

/// Indentation once grew with nesting depth, so output was quadratic in the
/// input: 50,000 nested blocks (100 KB) produced over 10 GB and failed after
/// 30 s at 32 GB RSS. Each line is now indented at most
/// `MAX_INDENT_DEPTH` levels.
#[test]
fn deep_indentation_keeps_output_linear() {
    let widest = 4 * jsbeautify::beautifier::MAX_INDENT_DEPTH;
    let blocks = indented_shapes(50_000).swap_remove(0);
    // Phase 12.5 local renaming is super-linear in nested function scopes
    // (22 s for 2,000 nested arrows in release), so arrow sequences only
    // run in `deep_indentation_keeps_behavior`.
    let shapes = indented_shapes(2_000)
        .into_iter()
        .skip(1)
        .filter(|&(shape, ..)| shape != "arrow sequences");
    for (shape, source, _) in std::iter::once(blocks).chain(shapes) {
        for args in [&["-"][..], &["-d", "-"]] {
            let out = jsbeautify(args, &source);
            let indent = out
                .lines()
                .map(|line| line.len() - line.trim_start().len())
                .max()
                .unwrap_or(0);
            assert!(indent <= widest, "{shape} {args:?}: indent {indent}");
            assert!(
                out.len() <= (widest + 16) * source.len(),
                "{shape} {args:?}: {} bytes from {}",
                out.len(),
                source.len()
            );
            assert_reparses(shape, &out);
        }
    }
}

/// Past `MAX_INDENT_DEPTH` the output is laid out without nesting
/// indentation; it must still behave like the input.
#[test]
fn deep_indentation_keeps_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-deep-indent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for (shape, source, expected) in indented_shapes(300) {
        let Some(input_stdout) = node_stdout(&dir, &source) else {
            eprintln!("node not found; skipping the behavior comparison");
            break;
        };
        assert_eq!(input_stdout.trim_end(), expected, "{shape}");
        for args in [&["-"][..], &["-d", "-"]] {
            let out = jsbeautify(args, &source);
            assert_eq!(
                node_stdout(&dir, &out).as_deref().map(str::trim_end),
                Some(expected),
                "{shape} {args:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
