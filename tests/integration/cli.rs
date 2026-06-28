use std::process::Command;

#[test]
fn help_exposes_bundle_source_splitting_without_active_bug_hunter_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("--help")
        .output()
        .expect("run jsbeautify --help");

    assert!(output.status.success(), "help failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("help is utf8");

    assert!(stdout.contains("--split-sources"));
    for flag in ["--agent", "--vuln-scan", "--validate", "--repeat", "--fuzz", "--report"] {
        assert!(
            !stdout.contains(flag),
            "stale active bug-hunter flag still in help: {flag}"
        );
    }
}
