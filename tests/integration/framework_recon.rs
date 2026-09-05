use std::process::Command;

#[test]
fn framework_recon_emits_json_without_executing_input() {
    let dir = std::env::temp_dir().join(format!("jsb-framework-recon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let input = dir.join("bundle.js");
    let output_path = dir.join("report.json");
    let sentinel = dir.join("executed.txt");
    std::fs::write(
        &input,
        format!(
            r#"require("fs").writeFileSync("{}", "executed");
               const login = createAction("[Auth] Login");
               client.variation("checkout-enabled", false);"#,
            sentinel.display()
        ),
    )
    .expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("framework-recon")
        .arg(&input)
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run framework reconnaissance");

    assert!(output.status.success(), "framework reconnaissance failed: {output:?}");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output_path).expect("read report")).expect("valid JSON report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["feature_flags"]["flags"][0]["name"], "checkout-enabled");
    assert_eq!(report["ngrx"]["actions"][0]["name"], "[Auth] Login");
    assert!(!sentinel.exists(), "framework reconnaissance executed JavaScript");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn framework_recon_rejects_over_limit_file_before_analysis() {
    let dir = std::env::temp_dir().join(format!("jsb-framework-recon-limit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let input = dir.join("oversized.js");
    let file = std::fs::File::create(&input).expect("create sparse input");
    file.set_len((32 * 1024 * 1024 + 1) as u64).expect("size sparse input");

    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .arg("framework-recon")
        .arg(&input)
        .output()
        .expect("run framework reconnaissance");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(
        stderr.contains("framework-recon input exceeds") && stderr.contains("byte limit"),
        "stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
