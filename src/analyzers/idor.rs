//! IDOR (Insecure Direct Object Reference) candidate detection.
//!
//! IDOR / broken object-level authorization (CWE-639, OWASP A01) is a *logic*
//! bug with no signature — you can't grep for it. But the structural tell is
//! recognizable: an API endpoint that takes a **client-supplied object
//! reference** — a numeric/uuid path segment or an `id`-like query parameter —
//! is a place where the server *might* forget to check that the caller owns the
//! referenced object. We surface those endpoints as `idor_candidate` leads for
//! manual testing, not as confirmed bugs.
//!
//! Heuristics (over endpoint strings the path/api analyzers already extract):
//!  - a path segment that is purely numeric (`/accounts/12345`),
//!  - a path segment that is a UUID,
//!  - a templated id segment (`/users/${id}`, `/users/:id`, `/users/{id}`),
//!  - a query parameter whose name ends in `id`/`Id` or is `user`/`account`/`uid`.

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;

/// Scan endpoint-like strings in `source` for IDOR candidates.
///
/// Operates on the same string-literal surface as the path/api extractors so it
/// can run standalone; callers usually run it alongside `run_all`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (lit, start) in string_literals(source) {
        if let Some(reason) = idor_reason(&lit)
            && seen.insert(lit.clone())
        {
            let position = PositionRange::from_byte_range(source, start, start + lit.len());
            out.push(
                AnalyzerMatch::new(
                    "idor_candidate",
                    lit.clone(),
                    position,
                    file_type,
                    file_path.map(String::from),
                )
                .with_provenance(Provenance::Derived)
                .with_severity(Severity::Low)
                .with_metadata(serde_json::json!({ "reason": reason })),
            );
        }
    }
    out
}

/// Classify a single endpoint string, returning the IDOR reason if it looks
/// like a direct-object-reference endpoint, else `None`.
#[must_use]
pub fn idor_reason(endpoint: &str) -> Option<&'static str> {
    // Must look like a path/URL with at least one `/` and an api-ish shape.
    let path = endpoint.split(['?', '#']).next().unwrap_or(endpoint);
    if !path.starts_with('/') && !path.contains("/api") && !endpoint.contains("://") {
        // Not obviously an endpoint; only consider if it has a query id below.
        if !has_id_query_param(endpoint) {
            return None;
        }
    }

    // A reference must sit under at least one named collection segment, e.g.
    // `/accounts/123` — not just `/123`. This cuts version numbers like `/v2/`.
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for (i, seg) in segments.iter().enumerate() {
        if i == 0 {
            continue; // need a collection name before the reference
        }
        let prev = segments[i - 1];
        if is_collection_name(prev) {
            if is_numeric_id(seg) {
                return Some("numeric path id under a collection");
            }
            if is_uuid(seg) {
                return Some("uuid path id under a collection");
            }
            if is_template_id(seg) {
                return Some("templated id path segment under a collection");
            }
        }
    }

    if has_id_query_param(endpoint) {
        return Some("id-like query parameter");
    }
    None
}

/// A segment that names a collection of objects (so the next segment is a ref).
fn is_collection_name(seg: &str) -> bool {
    let s = seg.trim_end_matches('s').to_ascii_lowercase();
    // Don't treat api/version markers as collections.
    if matches!(
        seg.to_ascii_lowercase().as_str(),
        "api" | "v1" | "v2" | "v3" | "rest" | "graphql"
    ) {
        return false;
    }
    // Heuristic: a lowercase alphabetic word ≥3 chars (users, accounts, orders…).
    !s.is_empty() && seg.chars().all(|c| c.is_ascii_alphabetic()) && seg.len() >= 3
}

fn is_numeric_id(seg: &str) -> bool {
    !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())
}

fn is_uuid(seg: &str) -> bool {
    // 8-4-4-4-12 hex.
    let parts: Vec<&str> = seg.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            == [
                parts[0].len(),
                parts[1].len(),
                parts[2].len(),
                parts[3].len(),
                parts[4].len(),
            ]
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_template_id(seg: &str) -> bool {
    // `${id}`, `:id`, `{id}`, `{accountId}` …
    let inner = seg
        .trim_start_matches("${")
        .trim_start_matches(':')
        .trim_start_matches('{')
        .trim_end_matches('}');
    (seg.starts_with("${") || seg.starts_with(':') || seg.starts_with('{'))
        && (inner.eq_ignore_ascii_case("id") || inner.to_ascii_lowercase().ends_with("id"))
}

fn has_id_query_param(endpoint: &str) -> bool {
    let Some(query) = endpoint.split('?').nth(1) else {
        return false;
    };
    for pair in query.split('&') {
        let name = pair.split('=').next().unwrap_or(pair).to_ascii_lowercase();
        if name.ends_with("id") || matches!(name.as_str(), "user" | "account" | "uid" | "uuid" | "guid") {
            return true;
        }
    }
    false
}

/// Minimal string-literal extractor (single/double/backtick).
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
                    b'\\' => j += 2,
                    c if c == q => break,
                    _ => j += 1,
                }
            }
            if j > value_start
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

    fn values(src: &str) -> Vec<String> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .map(|m| m.match_value)
            .collect()
    }

    #[test]
    fn flags_numeric_path_id() {
        assert!(idor_reason("/api/v2/accounts/12345").is_some());
        assert!(idor_reason("/users/42/profile").is_some());
    }

    #[test]
    fn flags_template_and_uuid_ids() {
        assert!(idor_reason("/users/${id}/orders").is_some());
        assert!(idor_reason("/users/:userId").is_some());
        assert!(idor_reason("/accounts/550e8400-e29b-41d4-a716-446655440000").is_some());
    }

    #[test]
    fn flags_id_query_param() {
        assert!(idor_reason("/api/profile?userId=7").is_some());
        assert!(idor_reason("/data?account=12").is_some());
    }

    #[test]
    fn ignores_version_and_static_paths() {
        assert!(idor_reason("/api/v2/users").is_none(), "collection root is not a ref");
        assert!(idor_reason("/static/app.js").is_none());
        assert!(idor_reason("/v1/health").is_none());
        assert!(idor_reason("hello world").is_none());
    }

    #[test]
    fn analyze_extracts_from_source() {
        let src = r#"fetch("/api/v2/accounts/98765"); get("/users/${id}");"#;
        let v = values(src);
        assert!(v.iter().any(|s| s.contains("accounts/98765")), "{v:?}");
        assert!(v.iter().any(|s| s.contains("users/${id}")), "{v:?}");
    }
}
