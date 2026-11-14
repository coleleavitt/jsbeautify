//! Declarations left dead by deobfuscation: once every decoder call is
//! inlined, the decoder function and its string array are removed, unless the
//! input is a classic script whose top-level bindings other scripts may read.

use std::path::Path;
use std::process::Command;

fn deobfuscate(dir: &Path, name: &str, code: &str, args: &[&str]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, code).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("-d")
        .args(args)
        .arg(&path)
        .output()
        .expect("run jsbeautify -d");
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
    String::from_utf8(output.stdout).expect("stdout is utf8")
}

/// What `code` prints under node, or `None` when node is not installed.
fn node_stdout(dir: &Path, name: &str, code: &str) -> Option<String> {
    let path = dir.join(name);
    std::fs::write(&path, code).expect("write script");
    let output = match Command::new("node").arg(&path).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("run node: {error}"),
    };
    assert!(output.status.success(), "node failed on {name}: {output:?}");
    Some(String::from_utf8(output.stdout).expect("stdout is utf8"))
}

fn assert_same_behavior(dir: &Path, input: &str, output: &str) {
    let Some(expected) = node_stdout(dir, "input_run.js", input) else {
        eprintln!("node not found; skipping the behavior comparison");
        return;
    };
    assert_eq!(
        node_stdout(dir, "output_run.js", output).as_deref(),
        Some(expected.as_str())
    );
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jsb-dead-decl-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn isolated_readme_example_keeps_only_the_decoded_statements() {
    let dir = temp_dir("readme");
    let input = include_str!("../fixtures/obfuscator/readme_example.js");
    let out = deobfuscate(&dir, "input.js", input, &["--isolated"]);
    assert_eq!(
        out.trim_end(),
        "var var_1 = [];\nvar_1.push(\"Hello\");\nvar_1.push(\"world\");\nconsole.log(var_1.join(\" \"), true);"
    );
    assert_same_behavior(&dir, input, &out);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn classic_script_keeps_its_top_level_decoder_and_array() {
    let dir = temp_dir("script");
    let input = include_str!("../fixtures/obfuscator/readme_example.js");
    let out = deobfuscate(&dir, "input.js", input, &[]);
    assert!(out.contains("var _0x1a2b = ["), "{out}");
    assert!(out.contains("function _0xdec("), "{out}");
    assert_same_behavior(&dir, input, &out);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn classic_script_drops_a_function_local_decoder() {
    let dir = temp_dir("local");
    let input = r#"function main(){var _0x1a2b=["Hello","log"];function _0xdec(_0x3c4d){_0x3c4d=_0x3c4d-0x64;return _0x1a2b[_0x3c4d];}console[_0xdec(0x65)](_0xdec(0x64));}main();"#;
    let out = deobfuscate(&dir, "input.js", input, &[]);
    assert!(!out.contains("_0x1a2b") && !out.contains("_0xdec"), "{out}");
    assert!(out.contains(r#"console.log("Hello")"#), "{out}");
    assert!(out.contains("function main()"), "{out}");
    assert_same_behavior(&dir, input, &out);
    let _ = std::fs::remove_dir_all(&dir);
}
