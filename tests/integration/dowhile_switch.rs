//! Do-while-switch dispatchers: the pruner removes a case only when no call
//! can reach it. Each input below once lost a reachable case, because the
//! pass matched the state, its constants and its calls by name.

use std::path::{Path, PathBuf};
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

/// A dispatcher in a function scope. `case_a` is the body of its first
/// case; the loop throws instead of hanging when the state never reaches
/// `END`.
fn dispatcher(consts: &str, before: &str, case_a: &str, more_cases: &str, calls: &str) -> String {
    format!(
        r#"(function () {{
  var A = 1, B = 2, C = 3, END = 9{consts};
  {before}
  var F = function (s, a) {{
    var steps = 0;
    do {{
      if (++steps > 20) throw new Error("no exit");
      switch (s) {{
        case A: {case_a}
        case B: console.log("B"); s = END; break;
        case C: console.log("C"); s = END; break;{more_cases}
      }}
    }} while (s != END);
  }};
  {calls}
}})();
"#
    )
}

fn cases() -> Vec<(&'static str, String, &'static str)> {
    let call = "F(A, []);";
    vec![
        (
            "a block's `let` shadows a constant",
            dispatcher("", "", r#"{ console.log("A"); let B = 3; s = B; } break;"#, "", call),
            "A\nC\n",
        ),
        (
            "a later conditional write",
            dispatcher(
                "",
                "",
                r#"console.log("A"); s = B; if (a.length === 0) s = C; break;"#,
                "",
                call,
            ),
            "A\nC\n",
        ),
        (
            "a closure writes the state",
            dispatcher(
                "",
                "",
                r#"console.log("A"); s = B; var bump = function () { s = C; }; bump(); break;"#,
                "",
                call,
            ),
            "A\nC\n",
        ),
        (
            "direct eval writes the state",
            dispatcher("", "", r#"console.log("A"); s = B; eval("s = C"); break;"#, "", call),
            "A\nC\n",
        ),
        (
            "the case falls through",
            dispatcher("", "", r#"console.log("A"); s = END;"#, "", call),
            "A\nB\n",
        ),
        (
            "a write after break",
            dispatcher("", "", r#"console.log("A"); s = C; break; s = B;"#, "", call),
            "A\nC\n",
        ),
        (
            "an alias calls the dispatcher",
            dispatcher(
                "",
                "",
                r#"console.log("A"); s = END; break;"#,
                "",
                "F(A, []); var G = F; G(C, []);",
            ),
            "A\nC\n",
        ),
        (
            "a literal state",
            dispatcher(
                "",
                "",
                r#"console.log("A"); s = END; break;"#,
                "",
                "F(A, []); F(3, []);",
            ),
            "A\nC\n",
        ),
        (
            "a call inside with",
            dispatcher(
                "",
                "",
                r#"console.log("A"); s = END; break;"#,
                "",
                "F(A, []); with ({ A: 3 }) { F(A, []); }",
            ),
            "A\nC\n",
        ),
        (
            "arguments aliases the state",
            dispatcher("", "", r#"console.log("A"); s = B; arguments[0] = C; break;"#, "", call),
            "A\nC\n",
        ),
        (
            "a default case",
            dispatcher(
                ", K = 7",
                "",
                r#"console.log("A"); s = K; break;"#,
                "\n        default: console.log(\"default\"); s = C; break;",
                call,
            ),
            "A\ndefault\nC\n",
        ),
        (
            "a constant aliases a label",
            dispatcher(", K = C", "", r#"console.log("A"); s = K; break;"#, "", call),
            "A\nC\n",
        ),
        (
            "an Annex B function replaces a label",
            dispatcher(
                "",
                "var Q = A; { function A() {} }",
                r#"console.log("A"); s = END; break;"#,
                "",
                "F(Q, []);",
            )
            .replace("C = 3", "C = 1"),
            "C\n",
        ),
        (
            "a shadowed constant in a compound transition",
            dispatcher(
                ", K = 3, D = 4, X = 6",
                "function unrelated() { var K = 5; return K; }",
                r#"console.log("A"); s += K; break;"#,
                "\n        case D: console.log(\"D\"); s = END; break;\n        case X: console.log(\"X\"); s = END; break;",
                "F(A, []); console.log(unrelated());",
            ),
            "A\nD\n5\n",
        ),
    ]
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jsb-dowhile-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn keeps_every_reachable_case() {
    let dir = temp_dir("reachable");
    for (what, code, expected) in cases() {
        if let Some(stdout) = node_stdout(&dir, &code) {
            assert_eq!(stdout, expected, "{what}: input under node");
        }
        for flags in [&["-d"][..], &["-d", "--isolated"]] {
            let out = deobfuscate(&dir, &code, flags);
            if let Some(stdout) = node_stdout(&dir, &out) {
                assert_eq!(stdout, expected, "{what} {flags:?}:\n{out}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prunes_cases_no_call_reaches() {
    let dir = temp_dir("dead");
    let code = dispatcher(
        ", D = 4",
        "",
        r#"console.log("A"); s = B; break;"#,
        "\n        case D: console.log(\"dead\"); s = END; break;",
        "F(A, []);",
    );
    for flags in [&["-d"][..], &["-d", "--isolated"]] {
        let out = deobfuscate(&dir, &code, flags);
        assert!(out.contains("\"B\""), "{flags:?}:\n{out}");
        assert!(!out.contains("\"C\"") && !out.contains("dead"), "{flags:?}:\n{out}");
        if let Some(stdout) = node_stdout(&dir, &out) {
            assert_eq!(stdout, "A\nB\n", "{flags:?}:\n{out}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
