//! CWE + OWASP Top 10 taxonomy tagging for findings.
//!
//! Every analyzer emits a `match_kind` string (`dom_xss`, `open_redirect`,
//! `high_entropy_secret`, …). This module maps those kinds onto the two
//! industry-standard taxonomies so our output speaks the same language as every
//! other scanner and can be filtered/reported by weakness class:
//!
//! - **CWE** (MITRE Common Weakness Enumeration) — the weakness type, e.g.
//!   `CWE-79` (XSS), `CWE-601` (open redirect), `CWE-639` (IDOR).
//! - **OWASP Top 10 (2021)** — the risk category, e.g. `A03:2021-Injection`,
//!   `A01:2021-Broken Access Control`.
//!
//! The mapping is a pure local table (no network), applied as an enrichment
//! pass so it works for every analyzer without each one knowing about CWE.

use super::match_kind::AnalyzerMatch;

/// A CWE + OWASP classification for a finding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    /// CWE identifier, e.g. `"CWE-79"`.
    pub cwe: &'static str,
    /// Short CWE weakness name.
    pub cwe_name: &'static str,
    /// OWASP Top 10 2021 category id, e.g. `"A03:2021"`.
    pub owasp: &'static str,
    /// OWASP category name.
    pub owasp_name: &'static str,
}

/// Map a `match_kind` to its CWE + OWASP classification, if known.
///
/// `sensitive_property_name:*` and `custom:*` prefixes are matched on their stem.
#[must_use]
pub fn classify(match_kind: &str) -> Option<Classification> {
    // Prefix kinds first.
    if match_kind.starts_with("sensitive_property_name:") {
        return Some(SENSITIVE_PROPERTY);
    }
    let base = match_kind.split(':').next().unwrap_or(match_kind);
    Some(match base {
        // ── Injection / XSS (A03) ────────────────────────────────────────────
        "dom_xss" | "html_manipulation" | "dangerously_set_inner_html" => Classification {
            cwe: "CWE-79",
            cwe_name: "Improper Neutralization of Input During Web Page Generation (XSS)",
            owasp: "A03:2021",
            owasp_name: "Injection",
        },
        // ── Broken access control (A01) ──────────────────────────────────────
        "open_redirect" | "location_assignment" => Classification {
            cwe: "CWE-601",
            cwe_name: "URL Redirection to Untrusted Site (Open Redirect)",
            owasp: "A01:2021",
            owasp_name: "Broken Access Control",
        },
        "idor_candidate" => Classification {
            cwe: "CWE-639",
            cwe_name: "Authorization Bypass Through User-Controlled Key (IDOR)",
            owasp: "A01:2021",
            owasp_name: "Broken Access Control",
        },
        "proto_pollution" => Classification {
            cwe: "CWE-1321",
            cwe_name: "Improperly Controlled Modification of Object Prototype Attributes",
            owasp: "A08:2021",
            owasp_name: "Software and Data Integrity Failures",
        },
        // ── Sensitive data / secrets (A02 / A07) ─────────────────────────────
        "high_entropy_secret"
        | "aws_access_key"
        | "google_api_key"
        | "github_pat"
        | "private_key"
        | "basic_auth_url" => Classification {
            cwe: "CWE-798",
            cwe_name: "Use of Hard-coded Credentials",
            owasp: "A07:2021",
            owasp_name: "Identification and Authentication Failures",
        },
        // ── postMessage / onmessage (A04 insecure design / origin checks) ────
        "post_message" | "window_onmessage" => Classification {
            cwe: "CWE-345",
            cwe_name: "Insufficient Verification of Data Authenticity",
            owasp: "A04:2021",
            owasp_name: "Insecure Design",
        },
        // ── Known CVE in a dependency (A06) ──────────────────────────────────
        "known_cve" => Classification {
            cwe: "CWE-1035",
            cwe_name: "Using Components with Known Vulnerabilities",
            owasp: "A06:2021",
            owasp_name: "Vulnerable and Outdated Components",
        },
        _ => return None,
    })
}

const SENSITIVE_PROPERTY: Classification = Classification {
    cwe: "CWE-200",
    cwe_name: "Exposure of Sensitive Information to an Unauthorized Actor",
    owasp: "A04:2021",
    owasp_name: "Insecure Design",
};

/// Attach CWE + OWASP metadata to each finding in place (merging into any
/// existing `metadata` object). Findings whose kind has no mapping are left
/// untouched.
pub fn enrich(matches: &mut [AnalyzerMatch]) {
    for m in matches.iter_mut() {
        let Some(c) = classify(&m.match_kind) else {
            continue;
        };
        let taxonomy = serde_json::json!({
            "cwe": c.cwe,
            "cwe_name": c.cwe_name,
            "owasp": c.owasp,
            "owasp_name": c.owasp_name,
        });
        match m.metadata.as_mut() {
            Some(serde_json::Value::Object(obj)) => {
                if let serde_json::Value::Object(t) = taxonomy {
                    obj.extend(t);
                }
            }
            _ => m.metadata = Some(taxonomy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::match_kind::{FileType, PositionRange};

    #[test]
    fn maps_known_kinds() {
        assert_eq!(classify("dom_xss").unwrap().cwe, "CWE-79");
        assert_eq!(classify("open_redirect").unwrap().cwe, "CWE-601");
        assert_eq!(classify("idor_candidate").unwrap().cwe, "CWE-639");
        assert_eq!(classify("high_entropy_secret").unwrap().owasp, "A07:2021");
        assert_eq!(classify("known_cve").unwrap().owasp, "A06:2021");
        assert_eq!(classify("sensitive_property_name:passwd").unwrap().cwe, "CWE-200");
    }

    #[test]
    fn unknown_kind_is_none() {
        assert!(classify("hostname").is_none());
        assert!(classify("path").is_none());
    }

    #[test]
    fn enrich_merges_into_metadata() {
        let mut matches = vec![
            AnalyzerMatch::new("dom_xss", "x", PositionRange::new(1, 0, 1, 1), FileType::Js, None)
                .with_metadata(serde_json::json!({ "existing": true })),
            AnalyzerMatch::new("hostname", "a.com", PositionRange::new(1, 0, 1, 1), FileType::Js, None),
        ];
        enrich(&mut matches);

        let meta = matches[0].metadata.as_ref().unwrap();
        assert_eq!(meta["cwe"], "CWE-79");
        assert_eq!(meta["owasp"], "A03:2021");
        assert_eq!(meta["existing"], true, "existing metadata preserved");
        // Unmapped kind keeps no taxonomy.
        assert!(matches[1].metadata.is_none());
    }
}
