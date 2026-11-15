//! The string-array-factory dispatch inliner (`f()[k]` with a constant `k`)
//! resolves factories and index constants through bindings, not names, and
//! leaves alone the globals a classic script shares with other scripts. Each
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
    // a hoisted function called before the constant's definition
    (
        r#"function f(){return ["a","b","c"];} console.log(g()); var I = 1; console.log(g()); function g(){ return f()[I]; }"#,
        "undefined\nb",
    ),
    (
        r#"(function(){ function f(){return ["a","b","c"];} console.log(g()); var I = 1; console.log(g()); function g(){ return f()[I]; } })();"#,
        "undefined\nb",
    ),
    // sloppy-mode `arguments[0]` writes the parameter
    (
        r#"function f(){return ["a","b","c"];} function g(J){ J = 0; arguments[0] = 2; return f()[J]; } console.log(g(5));"#,
        "c",
    ),
    // a non-index key runs an `Array.prototype` getter that changes the shared array
    (
        r#"Object.defineProperty(Array.prototype, "x", { get() { this[0] = "z"; return 1; } }); function f(){ var a = ["a","b"]; f = function(){return a;}; return a; } var K = "x"; f()[K]; console.log(f()[0]);"#,
        "z",
    ),
    // doubles round past 2^53
    (
        r#"function f(){return ["a","b"];} var A = 9007199254740992; var B = A + 1 - 9007199254740992; console.log(f()[B]);"#,
        "a",
    ),
    // an earlier declarator in the same declaration calls a hoisted function
    (
        r#"function f(){return ["a","b","c"];} var K = g(), J = 1; function g(){ console.log(f()[J]); return 0; } g();"#,
        "undefined\nb",
    ),
    // the global object and global-scope code assign globals
    (
        r#"function f(){return ["a","b"];} N = 0; globalThis.N = 1; console.log(f()[N]);"#,
        "b",
    ),
    (
        r#"function f(){return ["a","b"];} N = 0; (0, eval)("N = 1"); console.log(f()[N]);"#,
        "b",
    ),
    (
        r#"function f(){return ["a","b"];} N = 0; Function("N = 1")(); console.log(f()[N]);"#,
        "b",
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
    // Only `--isolated` promises that no other script changes the globals.
    for (flags, decoded) in [(&["-d"][..], false), (&["-d", "--isolated"], true)] {
        let output = deobfuscate(&dir, DECODED, flags);
        assert_eq!(
            output.contains(r#"console.log("t5", "gX", "B5""#),
            decoded,
            "{flags:?}:\n{output}"
        );
        if let Some(stdout) = node_stdout(&dir, &output) {
            assert_eq!(stdout.trim_end(), "t5 gX B5 0", "{flags:?}\n{output}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the scripts print when run one after another in one realm, as a
/// page runs its `<script>` elements, or `None` when node is not installed.
fn node_scripts_stdout(dir: &Path, scripts: &[&str]) -> Option<String> {
    let mut paths = Vec::new();
    for (index, script) in scripts.iter().enumerate() {
        let path = dir.join(format!("script{index}.js"));
        std::fs::write(&path, script).expect("write script");
        paths.push(path);
    }
    let runner = "const vm = require('vm'), fs = require('fs'); const realm = vm.createContext({ console }); \
                  for (const file of process.argv.slice(1)) vm.runInContext(fs.readFileSync(file, 'utf8'), realm, { filename: file });";
    let output = match Command::new("node").arg("-e").arg(runner).args(&paths).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("run node: {error}"),
    };
    assert!(output.status.success(), "node failed: {output:?}");
    Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
}

/// A classic script's top-level factory and constants, and the undeclared
/// globals it assigns, are shared with the scripts that run after it.
#[test]
fn classic_script_keeps_the_globals_other_scripts_change() {
    let dir = std::env::temp_dir().join(format!("jsb-dispatch-scripts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let first =
        r#"function f(){return ["a","b"];} var I = 0; NF = 0; function show(){ console.log(f()[I], f()[NF]); }"#;
    let second = r#"show(); I = 1; NF = 1; f = function(){ return ["x","y"]; }; show();"#;
    let output = deobfuscate(&dir, first, &["-d"]);
    if let Some(expected) = node_scripts_stdout(&dir, &[first, second]) {
        assert_eq!(expected.trim_end(), "a a\ny y");
        assert_eq!(
            node_scripts_stdout(&dir, &[&output, second]),
            Some(expected),
            "{output}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A module this one re-exports runs first, and may call a hoisted function
/// of this one before its top-level definitions run.
#[test]
fn re_exported_module_runs_before_the_definitions() {
    let dir = std::env::temp_dir().join(format!("jsb-dispatch-modules-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let entry = r#"function f(){return ["a","b","c","d"];} var J = 1; function h(){ return f()[J]; } export { h }; export * from "./b.mjs"; console.log(h());"#;
    let dependency = r#"import { h } from "./a.mjs"; console.log(h());"#;
    let input = dir.join("input.mjs");
    std::fs::write(&input, entry).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("-d")
        .arg(&input)
        .output()
        .expect("run jsbeautify");
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
    let deobfuscated = String::from_utf8(output.stdout).expect("stdout is utf8");
    std::fs::write(dir.join("b.mjs"), dependency).expect("write dependency");
    let run = |code: &str| {
        std::fs::write(dir.join("a.mjs"), code).expect("write entry");
        let output = match Command::new("node").arg(dir.join("a.mjs")).output() {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => panic!("run node: {error}"),
        };
        assert!(output.status.success(), "node failed: {output:?}");
        Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
    };
    if let Some(expected) = run(entry) {
        assert_eq!(expected.trim_end(), "undefined\nb");
        assert_eq!(run(&deobfuscated), Some(expected), "{deobfuscated}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
