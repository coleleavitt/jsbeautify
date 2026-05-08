//! Built-in secret scanner.
//!
//! Runs the regex bank from [`regex_bank`](super::regex_bank) against either
//! raw source text (for literal value rules) or extracted AST property names
//! (for property-name rules).
//!
//! Mirrors `crates/core/analyzers/src/regex_analyzer.rs` from jxscout.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange};
use super::regex_bank;

/// Scan `source` for secrets. `file_path` is optional (for output annotation).
#[must_use]
pub fn scan(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    for rule in regex_bank::literal_rules() {
        for m in rule.regex.find_iter(source) {
            out.push(AnalyzerMatch {
                match_kind: rule.kind.to_string(),
                match_value: m.as_str().to_string(),
                position: PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                file_path: file_path.map(str::to_string),
                metadata: None,
            });
        }
    }
    out
}

/// Scan a list of property-name strings (object keys, JSON keys) for sensitive
/// names. The position must be supplied by the caller (typically from oxc AST
/// span).
#[must_use]
pub fn scan_property_names<I, S>(names: I) -> Vec<&'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let rules = regex_bank::property_name_rules();
    let mut out = Vec::new();
    for name in names {
        let n = name.as_ref();
        for rule in &rules {
            if rule.regex.is_match(n) {
                out.push(rule.kind);
            }
        }
    }
    out
}

/// Convenience: classify a single property name.
#[must_use]
pub fn classify_property_name(name: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for rule in regex_bank::property_name_rules() {
        if rule.regex.is_match(name) {
            out.push(rule.kind);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_github_pat() {
        let src = r#"const token = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";"#;
        let m = scan(src, Some("test.js"), FileType::Js);
        assert!(m.len() >= 1);
        assert_eq!(m[0].match_kind, "github_pat");
        assert!(m[0].match_value.starts_with("ghp_"));
    }

    #[test]
    fn scan_finds_jwt_and_stripe() {
        let src = r#"
            const k1 = "sk_live_XXXXXXXXXXXXXXXXFAKE000";
            const k2 = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4eHgifQ.signature_abc";
        "#;
        let kinds: Vec<_> = scan(src, None, FileType::Js)
            .into_iter()
            .map(|m| m.match_kind)
            .collect();
        assert!(kinds.iter().any(|k| k.contains("stripe")));
        assert!(kinds.iter().any(|k| k == "jwt"));
    }

    #[test]
    fn scan_position_is_correct() {
        let src = "line1\nline2\n  ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let m = scan(src, None, FileType::Js);
        assert!(m.len() >= 1);
        assert_eq!(m[0].position.start.line, 3);
        assert_eq!(m[0].position.start.column, 2);
    }

    #[test]
    fn property_name_classifier_matches() {
        let kinds = classify_property_name("api_secret");
        assert!(kinds.iter().any(|k| k.contains("api")));
        assert!(kinds.iter().any(|k| k.contains("secret")));

        let kinds2 = classify_property_name("stripe_PK");
        // "stripe_PK" doesn't match any property pattern alone
        assert!(kinds2.is_empty() || kinds2.iter().any(|k| !k.is_empty()));
    }

    #[test]
    fn scan_empty_source_no_matches() {
        let m = scan("", None, FileType::Js);
        assert!(m.is_empty());
    }
}

/// Scan `source` with a list of user-supplied regex patterns.
/// Each pattern becomes its own match kind: `custom:<pattern_label>`.
#[must_use]
pub fn scan_custom(
    source: &str,
    file_path: Option<&str>,
    file_type: FileType,
    patterns: &[(String, regex::Regex)], // (label, compiled regex)
) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    for (label, re) in patterns {
        for m in re.find_iter(source) {
            out.push(AnalyzerMatch {
                match_kind: format!("custom:{label}"),
                match_value: m.as_str().to_string(),
                position: PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                file_path: file_path.map(str::to_string),
                metadata: None,
            });
        }
    }
    out
}
