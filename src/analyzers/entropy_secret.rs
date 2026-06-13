//! Entropy-based secret detection.
//!
//! The regex secret bank catches *known* token shapes (`ghp_…`, `AKIA…`). This
//! analyzer is the complement: it flags **high-entropy string literals** that
//! *look* like credentials regardless of prefix — random API keys, signing
//! secrets, session tokens — while deliberately ignoring the low-entropy noise
//! that drowns naive scanners (URLs, human text, hex colors, base64 of short
//! ASCII, repeated chars).
//!
//! Method: for each quoted string literal, compute Shannon entropy (bits/char)
//! over the value and combine it with charset + length heuristics into a
//! confidence score. Only strings that clear the bar become `high_entropy_secret`
//! findings, each carrying its entropy + score in metadata so triagers can rank.
//!
//! This is intentionally a cheap lexical pass (string-literal scan), not a full
//! AST/dataflow analysis — it complements [`super::secret_scanner`].

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;

/// Minimum length before a string is even considered (short randoms are noise).
const MIN_LEN: usize = 20;
/// Maximum length we treat as a credential (longer ⇒ probably data/markup).
const MAX_LEN: usize = 200;
/// Shannon-entropy floor (bits per character) for a base64/hex-ish token.
const ENTROPY_FLOOR: f64 = 3.5;

/// Shannon entropy in bits per character.
#[must_use]
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    let mut total = 0u32;
    for &b in s.as_bytes() {
        counts[b as usize] += 1;
        total += 1;
    }
    let total_f = f64::from(total);
    let mut entropy = 0.0;
    for &c in &counts {
        if c > 0 {
            let p = f64::from(c) / total_f;
            entropy = p.mul_add(-p.log2(), entropy);
        }
    }
    entropy
}

/// Fraction of characters that are in the typical token charset
/// (`[A-Za-z0-9_\-+/=.]`). High for keys, lower for prose / URLs with `://`.
fn token_charset_ratio(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let good = s
        .bytes()
        .filter(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'/' | b'=' | b'.'))
        .count();
    good as f64 / s.len() as f64
}

/// Heuristics that strongly indicate a value is NOT a secret.
fn is_obvious_non_secret(s: &str) -> bool {
    // URLs / paths / markup / sentences.
    if s.contains("://") || s.contains("</") || s.contains("<svg") || s.contains(' ') {
        return true;
    }
    // Hex colors and pure-numeric ids.
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    // Mostly punctuation / very few distinct chars (e.g. "----------------").
    let distinct = {
        let mut seen = [false; 256];
        let mut n = 0u32;
        for &b in s.as_bytes() {
            if !seen[b as usize] {
                seen[b as usize] = true;
                n += 1;
            }
        }
        n
    };
    distinct < 8
}

/// Confidence in `[0,1]` that `value` is a credential. 0 means "skip".
#[must_use]
pub fn secret_confidence(value: &str) -> f64 {
    let len = value.len();
    if !(MIN_LEN..=MAX_LEN).contains(&len) || is_obvious_non_secret(value) {
        return 0.0;
    }
    let entropy = shannon_entropy(value);
    if entropy < ENTROPY_FLOOR {
        return 0.0;
    }
    let charset = token_charset_ratio(value);
    if charset < 0.85 {
        return 0.0;
    }
    // Normalize entropy (≈3.5 floor, ≈6.0 = very random) into [0,1] and blend
    // with the charset ratio and a mild length bonus.
    let entropy_score = ((entropy - ENTROPY_FLOOR) / (6.0 - ENTROPY_FLOOR)).clamp(0.0, 1.0);
    let len_bonus = ((len as f64 - MIN_LEN as f64) / 60.0).clamp(0.0, 1.0);
    let blended = 0.1f64.mul_add(len_bonus, 0.3f64.mul_add(charset, 0.6 * entropy_score));
    blended.clamp(0.0, 1.0)
}

/// Scan `source` for high-entropy string literals and emit candidate-secret
/// findings. Confidence ≥ 0.5 ⇒ medium severity, ≥ 0.75 ⇒ high.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    for (value, start) in string_literals(source) {
        let conf = secret_confidence(&value);
        if conf < 0.5 {
            continue;
        }
        let severity = if conf >= 0.75 { Severity::High } else { Severity::Medium };
        let position = PositionRange::from_byte_range(source, start, start + value.len());
        out.push(
            AnalyzerMatch::new(
                "high_entropy_secret",
                value.clone(),
                position,
                file_type,
                file_path.map(String::from),
            )
            .with_provenance(Provenance::Derived)
            .with_severity(severity)
            .with_metadata(serde_json::json!({
                "entropy": (shannon_entropy(&value) * 100.0).round() / 100.0,
                "confidence": (conf * 100.0).round() / 100.0,
                "length": value.len(),
            })),
        );
    }
    out
}

/// Yield `(value, byte_offset)` for each single/double/backtick string literal.
/// A deliberately small lexer — handles escapes, skips template `${…}` exprs by
/// treating the whole backtick body as one literal (good enough for entropy).
fn string_literals(source: &str) -> Vec<(String, usize)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let q = bytes[i];
        if q == b'"' || q == b'\'' || q == b'`' {
            let value_start = i + 1;
            let mut j = value_start;
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' => j += 2, // skip escaped char
                    c if c == q => break,
                    _ => j += 1,
                }
            }
            if j <= bytes.len()
                && j > value_start
                && let Ok(val) = std::str::from_utf8(&bytes[value_start..j.min(bytes.len())])
            {
                out.push((val.to_string(), value_start));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_rank_order() {
        assert!(shannon_entropy("aaaaaaaa") < 1.0);
        assert!(shannon_entropy("abcdefgh") > shannon_entropy("aaaaaaaa"));
        // A random-looking token has high entropy.
        assert!(shannon_entropy("aB3xZ9qL2mP7wK4nR8tV") > 3.5);
    }

    #[test]
    fn flags_high_entropy_token() {
        // A 40-char random secret in an assignment.
        let src = r#"const k = "aB3xZ9qL2mP7wK4nR8tVcD1eF5gH0jI6kM2nO4p";"#;
        let m = analyze(src, None, FileType::Js);
        assert_eq!(m.len(), 1, "expected one high-entropy finding, got {m:?}");
        assert_eq!(m[0].match_kind, "high_entropy_secret");
        assert!(m[0].severity == Severity::High || m[0].severity == Severity::Medium);
    }

    #[test]
    fn ignores_urls_and_prose_and_ids() {
        let src = r#"
            const u = "https://www.wikidata.org/entity/Q1224715";
            const s = "this is a perfectly normal sentence value";
            const n = "1234567890123456789012345";
            const c = "----------------------------";
        "#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.is_empty(), "low-entropy/structured strings should not flag: {m:?}");
    }

    #[test]
    fn ignores_short_strings() {
        let src = r#"const k = "abc123";"#;
        assert!(analyze(src, None, FileType::Js).is_empty());
    }

    #[test]
    fn confidence_is_bounded() {
        let c = secret_confidence("aB3xZ9qL2mP7wK4nR8tVcD1eF5gH0jI6kM2nO4p");
        assert!((0.0..=1.0).contains(&c) && c >= 0.5);
        assert_eq!(secret_confidence("short"), 0.0);
    }
}
