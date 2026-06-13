//! Shared helpers for loading on-disk test fixtures.

use std::path::PathBuf;

/// Absolute path to `tests/fixtures/`.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// Read an obfuscated-sample fixture by file name (e.g. `"simple_string_array.js"`).
pub fn obfuscated(name: &str) -> String {
    let path = fixtures_dir().join("obfuscated").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// Every obfuscated fixture as `(file_name, contents)` pairs.
pub fn all_obfuscated() -> Vec<(String, String)> {
    let dir = fixtures_dir().join("obfuscated");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "js") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let body = std::fs::read_to_string(&path).expect("read fixture");
            out.push((name, body));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn fixtures_directory_is_populated() {
    let all = all_obfuscated();
    assert!(!all.is_empty(), "expected obfuscated fixtures on disk");
    assert!(
        all.iter().any(|(n, _)| n == "simple_string_array.js"),
        "missing known fixture simple_string_array.js",
    );
}
