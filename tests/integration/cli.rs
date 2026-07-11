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
