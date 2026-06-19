use super::*;

#[test]
fn test_align_identical_code() {
    let aligner = CrossVersionAligner::with_default_config();
    let code = "var x = 1;\nvar y = 2;";
    let (aligned_src, aligned_tgt, stats) = aligner.align_sources(code, code);

    assert_eq!(stats.matched_statements, 2);
    assert_eq!(aligned_src, aligned_tgt);
}

#[test]
fn test_align_different_var_names() {
    let aligner = CrossVersionAligner::with_default_config();
    let source = "var abc = 1;";
    let target = "var xyz = 1;";

    let (aligned_src, aligned_tgt, stats) = aligner.align_sources(source, target);

    assert_eq!(stats.matched_statements, 1);
    assert_eq!(aligned_src, aligned_tgt);
}

#[test]
fn test_stable_names_applied() {
    let mut aligner = CrossVersionAligner::with_default_config();
    aligner.stable_names.insert("x".to_string(), "originalX".to_string());

    let source = "var x = 1;";
    let target = "var y = 1;";

    let (aligned_src, aligned_tgt, stats) = aligner.align_sources(source, target);

    assert!(aligned_src.contains("originalX"));
    assert!(aligned_tgt.contains("originalX"));
    assert_eq!(stats.matched_statements, 1);
}
