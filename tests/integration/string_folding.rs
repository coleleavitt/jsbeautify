//! String folding passes on nested and long inputs: the output behaves like
//! the input under node, and hostile depths finish in linear memory.

use std::io::Write;
use std::process::{Command, Stdio};

use jsbeautify::AstDeobfuscator;

/// Shapes `n` levels deep or long, each printing one line under node.
fn shapes(n: usize) -> Vec<(&'static str, String)> {
    let plus = |term: &str| vec![term; n].join(" + ");
    vec![
        (
            "nested templates",
            format!("var r = {}'z'{};\nconsole.log(r);\n", "`a${".repeat(n), "}`".repeat(n)),
        ),
        (
            "nested templates with bindings",
            format!(
                "function f() {{\n  const x = 'q';\n  return {}x{};\n}}\nconsole.log(f());\n",
                "`a${x}${".repeat(n),
                "}`".repeat(n)
            ),
        ),
        ("string chain", format!("console.log({});\n", plus("'ab'"))),
        (
            "binding chain",
            format!(
                "function f() {{\n  const x = 'q';\n  return {};\n}}\nconsole.log(f());\n",
                plus("x")
            ),
        ),
        (
            "parenthesized chain",
            format!(
                "function f() {{\n  const x = 'q';\n  return {}x{};\n}}\nconsole.log(f());\n",
                "x + (".repeat(n),
                ")".repeat(n)
            ),
        ),
        (
            "concat chain",
            format!("var y = 'b';\nconsole.log(''{});\n", ".concat(y, '-')".repeat(n)),
        ),
        (
            "nested concat",
            format!(
                "var y = 'b';\nconsole.log({}y{});\n",
                "'a'.concat(".repeat(n),
                ")".repeat(n)
            ),
        ),
    ]
}

fn deobfuscate(shape: &str, src: &str) -> String {
    AstDeobfuscator::new()
        .deobfuscate(src)
        .unwrap_or_else(|e| panic!("deobfuscate {shape}: {e}"))
}

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

#[test]
fn folded_strings_keep_behavior() {
    for (shape, src) in shapes(200) {
        let out = deobfuscate(shape, &src);
        let Some(expected) = node_stdout(&src) else {
            eprintln!("node is not installed; skipping the behavior comparison");
            return;
        };
        assert_eq!(node_stdout(&out), Some(expected), "{shape} behaves differently:\n{out}");
    }
}

/// Before the fix, 20,000 nested templates took 40 s and 35 GB, and the
/// other shapes grew quadratically in memory as well.
#[test]
fn hostile_string_folding_stays_linear() {
    for (shape, src) in shapes(20_000) {
        let out = deobfuscate(shape, &src);
        assert!(
            out.len() <= 8 * src.len(),
            "{shape}: {} output bytes from {}",
            out.len(),
            src.len()
        );
    }
}
