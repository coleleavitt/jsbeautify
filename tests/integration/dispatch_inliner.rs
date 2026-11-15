//! The string-array-factory dispatch inliner (`f()[k]` with a constant `k`)
//! resolves factories and index constants through bindings, not names. Each
//! case was miscompiled by the name-matched pass; its output must print what
//! the input prints under node.

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

/// Each input with what node prints for it.
const CASES: &[(&str, &str)] = &[
    // annex b block function
    (
        r#"{ function f(){return ["x","y"];} } console.log(f()[0]); function f(){return ["a","b"];}"#,
        "x",
    ),
    // catch shadows constant
    (
        r#"var I = 0; function f(){return ["a","b","c"];} try { throw 2; } catch (I) { console.log(f()[I]); } console.log(f()[I]);"#,
        "c\na",
    ),
    // constant updated
    (
        r#"var J = 0; function f(){return ["a","b","c"];} J++; console.log(f()[J]);"#,
        "b",
    ),
    // constant written later
    (
        r#"var J = 0; function f(){return ["a","b","c"];} console.log(f()[J]); J = 2; console.log(f()[J]);"#,
        "a\nc",
    ),
    // direct eval shadows factory
    (
        r#"function f(){return ["a","b"];} function g(){ eval("var f = function(){return ['z']}"); return f()[0]; } console.log(g(), f()[0]);"#,
        "z a",
    ),
    // direct eval writes constant
    (
        r#"function f(){return ["a","b"];} var I = 0; eval("I = 1"); console.log(f()[I]);"#,
        "b",
    ),
    // factory reassigned
    (
        r#"function f(){return ["a","b"];} f = function(){return ["x","y"];}; console.log(f()[0]);"#,
        "x",
    ),
    // inner function shadows factory
    (
        r#"function f(){return ["a","b"];} function g(){ function f(){return ["x","y"];} return f()[1]; } console.log(g(), f()[1]);"#,
        "y b",
    ),
    // let redeclares constant
    (
        r#"var K = 0; function f(){return ["a","b","c"];} { let K = 1; console.log(f()[K]); } console.log(f()[K]);"#,
        "b\na",
    ),
    // param shadows constant
    (
        r#"var I = 0; function f(){return ["a","b","c"];} function g(I){ return f()[I]; } console.log(g(1), g(2), f()[I]);"#,
        "b c a",
    ),
    // param shadows factory
    (
        r#"function f(){return ["a","b","c"];} function g(f){ return f()[1]; } console.log(g(function(){return ["x","y","z"];}), f()[0]);"#,
        "y a",
    ),
    // with body var
    (
        r#"function f(){return ["a","b"];} var o = {I: 5}; with (o) { var I = 1; } console.log(f()[I], o.I);"#,
        "undefined 1",
    ),
    // async factory
    (
        r#"async function f(){return ["a","b"];} console.log(f()[0]);"#,
        "undefined",
    ),
    // read before init
    (
        r#"function f(){return ["a","b"];} console.log(f()[I]); var I = 1;"#,
        "undefined",
    ),
    // shared array mutated
    (
        r#"function f(){ var a = ["a","b"]; f = function(){return a;}; return a; } f()[0]; f().reverse(); console.log(f()[0]);"#,
        "b",
    ),
    // compound assignment global
    (
        r#"function f(){return ["a","b","c"];} N = 1; N += 1; console.log(f()[N]);"#,
        "c",
    ),
    // factory param default
    (
        r#"var n = 0; function f(x = n++){return ["a","b"];} console.log(f()[0], n);"#,
        "a 1",
    ),
];

/// Factories and constants that nothing else can change are still decoded.
const DECODED: &str = r#"var I = 1; J = 2; function x19(){return ["BR","t5","B5"];} function Y49(){ var v = ["HR","tg","gX"]; Y49 = function(){return v;}; return v; } function show(I){ return I; } console.log(x19()[I], Y49()[J], x19()[I + 1], show(0));"#;

#[test]
fn shadowed_or_rewritten_bindings_keep_their_behavior() {
    let dir = std::env::temp_dir().join(format!("jsb-dispatch-{}", std::process::id()));
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

#[test]
fn decodes_factories_through_their_bindings() {
    let dir = std::env::temp_dir().join(format!("jsb-dispatch-decoded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for flags in [&["-d"][..], &["-d", "--isolated"]] {
        let output = deobfuscate(&dir, DECODED, flags);
        assert!(
            output.contains(r#"console.log("t5", "gX", "B5""#),
            "{flags:?} left a call site:\n{output}"
        );
        if let Some(stdout) = node_stdout(&dir, &output) {
            assert_eq!(stdout.trim_end(), "t5 gX B5 0", "{flags:?}\n{output}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
