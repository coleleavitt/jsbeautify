use std::process::Command;

#[test]
fn help_exposes_bundle_source_splitting_without_bug_hunter_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("--help")
        .output()
        .expect("run jsbeautify --help");

    assert!(output.status.success(), "help failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("help is utf8");

    // Bundle source splitting is a first-class subcommand.
    assert!(stdout.contains("split-sources"), "stdout: {stdout}");
    // The jxscout / recon / serve surface was cut — none of it should resurface.
    for flag in [
        "--analyze",
        "--wordlist",
        "--serve",
        "--crtsh",
        "--recon",
        "--sarif",
        "--agent",
        "--vuln-scan",
    ] {
        assert!(!stdout.contains(flag), "cut flag still in help: {flag}");
    }
}

#[test]
fn split_sources_splits_no_sourcemap_bundle_by_structure() {
    let dir = std::env::temp_dir().join(format!("jsb-cli-split-nosmtest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bundle_path = dir.join("bundle.js");
    std::fs::write(
        &bundle_path,
        "function alpha(){return 1;} class Beta { method(){ return alpha(); } }",
    )
    .expect("write bundle");
    let out_dir = dir.join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("split-sources")
        .arg(&bundle_path)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("run jsbeautify split-sources");

    assert!(output.status.success(), "split failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert!(stdout.contains("synthetic/functions/alpha.js"), "stdout: {stdout}");
    assert!(out_dir.join("reversed/synthetic/functions/alpha.js").exists());
    assert!(out_dir.join("reversed/synthetic/classes/Beta.js").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analyze_vm_emits_static_json_without_executing_input() {
    let dir = std::env::temp_dir().join(format!("jsb-cli-vm-analysis-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let input = dir.join("vm.js");
    let report = dir.join("report.json");
    let extracted = dir.join("wasm");
    let sentinel = dir.join("executed.txt");
    std::fs::write(
        &input,
        format!(
            r#"
            require("fs").writeFileSync("{}", "executed");
            function decodeBase64(value) {{ return atob(value); }}
            const bytes = decodeBase64("AGFzbQEAAAABBQFgAAF/AwIBAAcFAQFmAAAKBgEEAEEHCw==");
            const module = new WebAssembly.Module(bytes);
            "#,
            sentinel.display()
        ),
    )
    .expect("write VM fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("analyze-vm")
        .arg(&input)
        .arg("--output")
        .arg(&report)
        .arg("--extract-wasm")
        .arg(&extracted)
        .output()
        .expect("run jsbeautify analyze-vm");

    assert!(output.status.success(), "VM analysis failed: {output:?}");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).expect("read report")).expect("valid JSON report");
    assert_eq!(value["wasm_modules"].as_array().map(Vec::len), Some(1));
    assert!(
        value["patterns"]
            .as_array()
            .is_some_and(|patterns| { patterns.iter().any(|finding| finding["kind"] == "web_assembly_bridge") })
    );
    let module_path = extracted.join("module-000.wasm");
    let original_module = std::fs::read(&module_path).expect("read extracted WASM");
    assert_eq!(original_module[..4], *b"\0asm");
    assert!(!sentinel.exists(), "analyze-vm executed target JavaScript");

    let second = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("analyze-vm")
        .arg(&input)
        .arg("--output")
        .arg(&report)
        .arg("--extract-wasm")
        .arg(&extracted)
        .output()
        .expect("rerun analyze-vm with existing artifact");
    assert!(
        second.status.success(),
        "existing extraction should be skipped: {second:?}"
    );
    assert_eq!(std::fs::read(&module_path).expect("reread module"), original_module);
    assert!(!sentinel.exists(), "rerun executed target JavaScript");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analyze_vm_rejects_over_limit_file_before_reading_it() {
    let dir = std::env::temp_dir().join(format!("jsb-cli-vm-over-limit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let input = dir.join("oversized.js");
    let file = std::fs::File::create(&input).expect("create sparse input");
    file.set_len((32 * 1024 * 1024 + 1) as u64).expect("size sparse input");

    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("analyze-vm")
        .arg(&input)
        .output()
        .expect("run analyze-vm over limit");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(
        stderr.contains("exceeds") && stderr.contains("byte limit"),
        "stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn deobfuscate_file(name: &str, code: &str, args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("jsb-cli-goal-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    std::fs::write(&path, code).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("-d")
        .args(args)
        .arg(&path)
        .output()
        .expect("run jsbeautify -d");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
    String::from_utf8(output.stdout).expect("stdout is utf8")
}

#[test]
fn source_type_and_isolated_decide_whether_globals_survive() {
    let code = "var unused = 1;";
    assert!(deobfuscate_file("a.js", code, &[]).contains("var unused"));
    assert!(!deobfuscate_file("b.js", code, &["--isolated"]).contains("unused"));
    assert!(!deobfuscate_file("c.js", code, &["--source-type", "module"]).contains("unused"));
    assert!(deobfuscate_file("d.mjs", code, &["--source-type", "script"]).contains("var unused"));
    assert!(!deobfuscate_file("e.cjs", code, &[]).contains("unused"));
    assert!(!deobfuscate_file("f.mjs", code, &[]).contains("unused"));
}

fn deobfuscate_stderr(name: &str, code: &str, trace: bool) -> String {
    let dir = std::env::temp_dir().join(format!("jsb-cli-log-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    std::fs::write(&path, code).expect("write input");
    let mut command = Command::new(env!("CARGO_BIN_EXE_jsbeautify"));
    command
        .arg("-d")
        .arg(&path)
        .env_remove("JSB_TRACE_DEOB")
        .env_remove("JSBEAUTIFY_TRACE_DEOB");
    if trace {
        command.env("JSB_TRACE_DEOB", "1");
    }
    let output = command.output().expect("run jsbeautify -d");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
    String::from_utf8(output.stderr).expect("stderr is utf8")
}

#[test]
fn deobfuscate_log_reports_counts_per_phase_not_lines_per_rewrite() {
    let code = "function f(a) { return [!0, void 0, a?.b, 2 * 3]; }\n".repeat(2000);

    let quiet = deobfuscate_stderr("many.js", &code, false);
    assert!(quiet.lines().count() < 300, "{} stderr lines", quiet.lines().count());
    assert!(
        quiet.contains("Phase 19.6: Generated output re-parse succeeded"),
        "{quiet}"
    );
    assert!(
        quiet.contains("Phase 21.5: Generated output re-parse succeeded"),
        "{quiet}"
    );
    assert!(
        quiet.contains("Phase 16.9: Detected 2000 optional chaining expressions"),
        "{quiet}"
    );
    assert!(quiet.contains("replaced 2000 void expressions"), "{quiet}");

    let traced = deobfuscate_stderr("many.js", &code, true);
    let per_node = traced
        .lines()
        .filter(|line| line.starts_with("[OPTIONAL_CHAINING]"))
        .count();
    assert_eq!(per_node, 2000, "JSB_TRACE_DEOB restores the per-node lines");
}

#[test]
fn bundler_checks_print_one_result_line_each() {
    let plain = deobfuscate_stderr("plain.js", "function add(a, b) { return a + b; }", false);
    for check in [
        "Akamai BMP check",
        "webpack check",
        "esbuild/Bun runtime check",
        "Angular check",
    ] {
        let results: Vec<_> = plain
            .lines()
            .filter(|line| line.contains(&format!("{check}: ")))
            .collect();
        assert_eq!(results.len(), 1, "{check}: {results:?}");
        assert!(results[0].contains(&format!("{check}: not found")), "{}", results[0]);
    }

    let chunk = "(self.webpackChunkapp = self.webpackChunkapp || []).push([[1], {\
                 10: function (e, t, n) { e.exports = 1; },\
                 11: function (e, t, n) { e.exports = n(10); }\
                 }]);";
    let stderr = deobfuscate_stderr("chunk.js", chunk, false);
    assert!(
        stderr.contains("Phase 0.85: webpack check: detected webpack chunk with 2 modules"),
        "{stderr}"
    );
}
