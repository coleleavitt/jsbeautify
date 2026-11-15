//! Akamai BMP passes: equality-proxy unwrapping and stack-tracker removal
//! resolve their bindings by symbol, so a binding of the same name that
//! shadows, reassigns or redeclares the proxy or tracker keeps its behavior.
//! `var bmak, _abck` make the detector run the Akamai passes.

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

/// Each input, what node prints for it and for its deobfuscated output, and
/// text the output must no longer contain.
const CASES: &[(&str, &str, &[&str])] = &[
    // eq_param_shadow
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; function g(BI) { return BI(1, 2); } console.log(g(function (a, b) { return a + b; }), BI(3, 3)); })();",
        "3 true",
        &[],
    ),
    // eq_local_shadow
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; function h() { var BI = function (a, b) { return a !== b; }; return BI(1, 1); } function k(x) { { let BI = function (a, b) { return a < b; }; return BI(x, 2); } } console.log(h(), k(1), BI(1, 1)); })();",
        "false true true",
        &[],
    ),
    // eq_reassigned
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; BI = function (a, b) { return a < b; }; console.log(BI(1, 2)); })();",
        "true",
        &[],
    ),
    // eq_redeclared
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; var BI = function (a, b) { return a + b; }; console.log(BI(1, 2)); })();",
        "3",
        &[],
    ),
    // eq_dup_params
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, a) { return a === a; }; console.log(BI(1, 2)); })();",
        "true",
        &[],
    ),
    // eq_eval
    (
        r#"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; eval("BI = function (a, b) { return a + b; }"); console.log(BI(1, 2)); })();"#,
        "3",
        &[],
    ),
    // eq_with
    (
        r"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; with ({ BI: function (a, b) { return a * b; } }) { console.log(BI(3, 4)); } })();",
        "12",
        &[],
    ),
    // tracker_param_shadow
    (
        r"var bmak = 1, _abck = 2; (function () { var Ot; Ot = []; Ot.push(1); Ot.push(2); Ot.pop(); Ot.pop(); function add(Ot, v) { Ot.push(v); } var out = []; add(out, 7); console.log(out.length); })();",
        "1",
        &[],
    ),
    // tracker_value_used
    (
        r"var bmak = 1, _abck = 2; (function () { var Ot; Ot = []; Ot.push(1); Ot.push(2); Ot.push(3); var x = (0, Ot.pop()); console.log(x); })();",
        "3",
        &[],
    ),
    // tracker_strict_assign
    (
        r#"var bmak = 1, _abck = 2; (function () { "use strict"; var Ot; Ot = []; Ot.push(1); Ot.push(2); Ot.push(3); Ot.pop(); console.log("ok"); })();"#,
        "ok",
        &[],
    ),
    // tracker_shared_array
    (
        r"var bmak = 1, _abck = 2; var shared = []; (function () { var Ot; Ot = shared; Ot.push(1); Ot.push(2); Ot.push(3); Ot.push(4); })(); console.log(shared.length);",
        "4",
        &[],
    ),
    // tracker_side_effect_arg
    (
        r"var bmak = 1, _abck = 2; (function () { var Ot, n = 0; Ot = []; function tick() { n++; return n; } Ot.push(tick()); Ot.push(tick()); Ot.pop(); Ot.pop(); console.log(n); })();",
        "2",
        &[],
    ),
    // eq_positive
    (
        r#"var bmak = 1, _abck = 2; (function () { var BI = function (a, b) { return a === b; }; function c8(a, b) { return a !== b; } var y; function t(v) { return BI(typeof v, "undefined") + "," + c8(v, 1); } console.log(t(y), t(1), BI(typeof q, "undefined")); })();"#,
        "true,true false,false true",
        &["BI(", "c8("],
    ),
    // tracker_positive
    (
        r"var bmak = 1, _abck = 2; (function () { var Ot; Ot = []; function f(Zl) { Ot.push(Zl); var r = Zl * 2; return Ot.pop(), r; } function g(Zl) { Ot.push(Zl); try { return f(Zl) + 1; } finally { Ot.splice(0, 1, Zl); Ot.pop(); } } console.log(f(2), g(3)); })();",
        "4 7",
        &["Ot"],
    ),
];

#[test]
fn akamai_passes_keep_behavior_under_shadowing() {
    let dir = std::env::temp_dir().join(format!("jsb-akamai-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["-d", "--isolated"]] {
        for &(input, expected, absent) in CASES {
            let output = deobfuscate(&dir, input, flags);
            for text in absent {
                assert!(!output.contains(text), "{flags:?} {input}\n=>\n{output}");
            }
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
