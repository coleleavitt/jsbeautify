//! js-beautify-compatible formatting flags, config files and file handling.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("jsb-format-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create sandbox");
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        std::fs::write(&path, text).expect("write file");
        path
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn jsbeautify");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait for jsbeautify")
    }

    fn stdout(&self, args: &[&str], stdin: &str) -> String {
        let output = self.run(&self.0, args, stdin);
        assert!(output.status.success(), "{args:?} failed: {output:?}");
        String::from_utf8(output.stdout).expect("utf8 stdout")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const NESTED: &str = "if(a){if(b){c()}}";

#[test]
fn default_output_matches_js_beautify_defaults() {
    let sandbox = Sandbox::new("defaults");
    assert_eq!(
        sandbox.stdout(&[], NESTED),
        "if (a) {\n    if (b) {\n        c();\n    }\n}"
    );
}

#[test]
fn indent_size_flags() {
    let sandbox = Sandbox::new("indent-size");
    let expected = "if (a) {\n  if (b) {\n    c();\n  }\n}";
    for args in [
        ["-s", "2"],
        ["--indent-size", "2"],
        ["-i", "2"],
        ["--indent", "2"],
        ["--indent_size", "2"],
    ] {
        assert_eq!(sandbox.stdout(&args, NESTED), expected, "{args:?}");
    }
}

#[test]
fn indent_char_and_tabs() {
    let sandbox = Sandbox::new("indent-char");
    let tabs = "if (a) {\n\tif (b) {\n\t\tc();\n\t}\n}";
    assert_eq!(sandbox.stdout(&["-c", "\t"], NESTED), tabs);
    assert_eq!(sandbox.stdout(&["--indent-char", "\\t"], NESTED), tabs);
    assert_eq!(sandbox.stdout(&["-t"], NESTED), tabs);
    assert_eq!(sandbox.stdout(&["--tabs"], NESTED), tabs);
    let output = sandbox.run(&sandbox.0, &["-c", "x"], NESTED);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("space or a tab"));
}

#[test]
fn indent_level_and_end_with_newline() {
    let sandbox = Sandbox::new("indent-level");
    assert_eq!(sandbox.stdout(&["-l", "1", "-s", "2"], "a()"), "  a();");
    assert_eq!(sandbox.stdout(&["--indent-level", "2", "-t"], "a()"), "\t\ta();");
    assert_eq!(sandbox.stdout(&[], "   a()"), "   a();");
    assert_eq!(sandbox.stdout(&["-n"], "a()"), "a();\n");
    assert_eq!(
        sandbox.stdout(&["--end-with-newline", "-e", "\\r\\n"], "a()"),
        "a();\r\n"
    );
}

#[test]
fn eol_flag() {
    let sandbox = Sandbox::new("eol");
    assert_eq!(sandbox.stdout(&["-e", "\\r\\n"], "{a()}"), "{\r\n    a();\r\n}");
    assert_eq!(sandbox.stdout(&["--eol", "\r"], "{a()}"), "{\r    a();\r}");
    assert_eq!(sandbox.stdout(&[], "{a()\r\n}"), "{\r\n    a();\r\n}");
    assert_eq!(sandbox.stdout(&["-e", "auto"], "{a()\r}"), "{\r    a();\r}");
    assert!(!sandbox.run(&sandbox.0, &["-e", "x"], "a").status.success());
}

#[test]
fn preserve_newlines_flags() {
    let sandbox = Sandbox::new("preserve");
    let input = "a();\n\n\n\nb();";
    assert_eq!(sandbox.stdout(&[], input), "a();\n\n\n\nb();");
    assert_eq!(sandbox.stdout(&["-p"], input), "a();\n\n\n\nb();");
    assert_eq!(sandbox.stdout(&["-m", "2"], input), "a();\n\nb();");
    assert_eq!(sandbox.stdout(&["--max-preserve-newlines", "1"], input), "a();\nb();");
    assert_eq!(sandbox.stdout(&["--no-preserve-newlines"], input), "a();\nb();");
}

#[test]
fn function_spacing_flags() {
    let sandbox = Sandbox::new("spacing");
    let input = "var f=function(){};function g(){}";
    assert_eq!(
        sandbox.stdout(&["-a"], input),
        "var f = function () {};\n\nfunction g() {}"
    );
    assert_eq!(
        sandbox.stdout(&["--space-after-named-function"], input),
        "var f = function() {};\n\nfunction g () {}"
    );
    assert_eq!(
        sandbox.stdout(&["-j"], "switch(a){case 1:b()}f(function(){})"),
        "switch (a) {\ncase 1:\n    b();\n}\nf(function () {});"
    );
}

#[test]
fn brace_style_flag() {
    let sandbox = Sandbox::new("brace-style");
    let input = "if(a){b()}else{c()}";
    assert_eq!(
        sandbox.stdout(&["-b", "collapse"], input),
        "if (a) {\n    b();\n} else {\n    c();\n}"
    );
    assert_eq!(
        sandbox.stdout(&["-b", "expand"], input),
        "if (a)\n{\n    b();\n}\nelse\n{\n    c();\n}"
    );
    assert_eq!(
        sandbox.stdout(&["--brace-style", "end-expand"], input),
        "if (a) {\n    b();\n}\nelse {\n    c();\n}"
    );
    assert!(!sandbox.run(&sandbox.0, &["-b", "none"], input).status.success());
}

#[test]
fn files_replace_and_quiet() {
    let sandbox = Sandbox::new("files");
    let a = sandbox.write("a.js", "a()");
    let b = sandbox.write("b.js", "b()");
    let output = sandbox.run(&sandbox.0, &["a.js", "-f", "b.js"], "");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "several inputs are replaced in place");
    assert!(String::from_utf8_lossy(&output.stderr).contains("beautified a.js"));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a();");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b();");

    let c = sandbox.write("c.js", "c()");
    let output = sandbox.run(&sandbox.0, &["-r", "-q", "c.js"], "");
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "c();");

    let output = sandbox.run(&sandbox.0, &["c.js", "-o", "out.js"], "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("beautified out.js"));
    let output = sandbox.run(&sandbox.0, &["c.js", "-o", "out.js"], "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("unchanged"));
}

#[test]
fn config_files_and_precedence() {
    let sandbox = Sandbox::new("config");
    sandbox.write(
        ".jsbeautifyrc",
        r#"{"indent_size": 2, "brace_style": "end-expand,preserve-inline", "js": {"end-with-newline": true}, "wrap_attributes": "force"}"#,
    );
    let nested = sandbox.0.join("deep/er");
    std::fs::create_dir_all(&nested).unwrap();
    let output = sandbox.run(&nested, &[], "if(a){b()}else{c()}");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "if (a) {\n  b();\n}\nelse {\n  c();\n}\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("preserve-inline"));

    let explicit = sandbox.write("explicit.json", r#"{"indent_size": "8", "end_with_newline": false}"#);
    let explicit = explicit.to_str().unwrap();
    assert_eq!(sandbox.stdout(&["--config", explicit], "{a()}"), "{\n        a();\n}");
    assert_eq!(
        sandbox.stdout(&["--config", explicit, "-s", "3"], "{a()}"),
        "{\n   a();\n}"
    );

    let home_only = Sandbox::new("config-home");
    home_only.write(".jsbeautifyrc", r#"{"indent_with_tabs": true}"#);
    assert_eq!(home_only.stdout(&[], "{a()}"), "{\n\ta();\n}");

    let bad = sandbox.write("bad.json", r#"{"indent_size": true}"#);
    assert!(
        !sandbox
            .run(&sandbox.0, &["--config", bad.to_str().unwrap()], "a")
            .status
            .success()
    );
}

#[test]
fn editorconfig_overrides_flags() {
    let sandbox = Sandbox::new("editorconfig");
    sandbox.write(
        ".editorconfig",
        "root = true\n[*.js]\nindent_style = tab\nend_of_line = crlf\ninsert_final_newline = true\n",
    );
    sandbox.write("x.js", "{a()}");
    assert_eq!(
        sandbox.stdout(&["x.js", "--editorconfig", "-s", "2"], ""),
        "{\r\n\ta();\r\n}\r\n"
    );
    assert_eq!(sandbox.stdout(&["--editorconfig"], "{a()}"), "{\r\n\ta();\r\n}\r\n");
    assert_eq!(sandbox.stdout(&["x.js"], ""), "{\n    a();\n}");
}

#[test]
fn version_short_flags() {
    let sandbox = Sandbox::new("version");
    assert!(sandbox.stdout(&["-v"], "").starts_with("jsbeautify "));
    assert!(sandbox.stdout(&["-V"], "").starts_with("jsbeautify "));
}

#[test]
fn deobfuscate_honors_layout_flags() {
    let sandbox = Sandbox::new("deobfuscate");
    let out = sandbox.stdout(&["-d", "-s", "2", "-n"], "function f(){if(a){return 1}}f()");
    assert!(out.contains("\n  if (a) {\n    return 1;\n  }"), "{out}");
    assert!(out.ends_with("}\n") || out.ends_with(";\n"), "{out:?}");
}

#[test]
fn deobfuscation_trade_offs_are_opt_in_flags_and_config_keys() {
    let names = "var a=function(){};export{a as useThing};";
    let alias = "var w=window;export const p=w.Promise,d=w.document;";
    let short = "var r=1;export{r as ab};";
    let sandbox = Sandbox::new("deob-flags");
    let default = sandbox.stdout(&["-d", "--source-type", "module"], &format!("{names}{alias}{short}"));
    assert!(default.contains("var useThing = function"), "{default}");
    assert!(default.contains("p = Promise"), "{default}");
    assert!(default.contains("r as ab"), "{default}");

    let flagged = sandbox.stdout(
        &[
            "-d",
            "--source-type",
            "module",
            "--keep-fn-names",
            "--window-alias-es5",
            "--short-export-names",
        ],
        &format!("{names}{alias}{short}"),
    );
    assert!(flagged.contains("var a = function"), "{flagged}");
    assert!(
        flagged.contains(".Promise,") && flagged.contains("d = document"),
        "{flagged}"
    );
    assert!(flagged.contains("var ab = 1"), "{flagged}");

    let configured = Sandbox::new("deob-config");
    configured.write(
        ".jsbeautifyrc",
        r#"{"keep_fn_names": true, "window-alias-es5": true, "js": {"short_export_names": true}}"#,
    );
    assert_eq!(
        configured.stdout(&["-d", "--source-type", "module"], &format!("{names}{alias}{short}")),
        flagged
    );
}
