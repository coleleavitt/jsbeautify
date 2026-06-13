//! Offline vulnerability-catalog lookup: known CVEs for detected components.
//!
//! Our analyzers detect *which* libraries a page ships and at what `version`
//! (jQuery 3.7.1, AngularJS 1.8.2, the Okta widget 6.9.0, …). This module turns
//! those into **known-CVE findings** by consulting the public catalogs where the
//! world's disclosed vulnerabilities live:
//!
//! - **OSV** (<https://osv.dev>) — Google's open-source vuln DB; one batch HTTP
//!   query maps `{name, version, ecosystem}` → the CVEs/GHSAs that affect it.
//! - **CISA KEV** (<https://www.cisa.gov/.../known_exploited_vulnerabilities.json>)
//!   — the catalog of CVEs *actually exploited in the wild*; we flag any matched
//!   CVE that appears here as `kev: true` so it sorts to the top.
//!
//! The HTTP layer is thin and behind a flag; the **mapping/merge logic is pure
//! and unit-tested** without network so the catalog behavior is verifiable
//! offline. Output is a list of `known_cve` findings (CVE id, summary, severity,
//! KEV flag) ready to feed back through [`crate::analyzers::taxonomy`].
#![allow(clippy::doc_markdown)] // module docs name catalogs (OSV, CISA KEV)

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;

/// A detected component to look up (name + version + ecosystem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub version: String,
    /// OSV ecosystem, e.g. `"npm"`. Most web libs are npm.
    pub ecosystem: String,
}

impl Component {
    #[must_use]
    pub fn npm(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            ecosystem: "npm".to_string(),
        }
    }
}

/// A known vulnerability affecting a component.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KnownVuln {
    pub component: String,
    pub version: String,
    /// Preferred id: the CVE if present, else the OSV/GHSA id.
    pub id: String,
    /// All aliases (CVE-…, GHSA-…).
    pub aliases: Vec<String>,
    pub summary: String,
    /// Whether the CVE is in CISA's Known Exploited Vulnerabilities catalog.
    pub kev: bool,
}

// ── OSV response model (only the fields we use) ──────────────────────────────

#[derive(Debug, Deserialize)]
struct OsvBatchResponse {
    results: Vec<OsvResult>,
}

/// One component's OSV result (the vulns affecting it). Public so the pure
/// [`merge_results`] core can be exercised in tests + by callers.
#[derive(Debug, Deserialize, Default)]
pub struct OsvResult {
    #[serde(default)]
    vulns: Vec<OsvVuln>,
}
#[derive(Debug, Deserialize)]
struct OsvVuln {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: String,
}

/// Pick the best display id: prefer a CVE alias, else the OSV id.
fn preferred_id(osv_id: &str, aliases: &[String]) -> String {
    aliases
        .iter()
        .find(|a| a.starts_with("CVE-"))
        .cloned()
        .unwrap_or_else(|| osv_id.to_string())
}

/// Merge OSV results (parallel to `components`) + a KEV id set into findings.
///
/// Pure: this is the testable core. `osv_json` is the raw OSV `/v1/querybatch`
/// **or** `/v1/query` per-component response bodies, already deserialized.
#[must_use]
pub fn merge_results(components: &[Component], osv: &[OsvResult], kev: &BTreeSet<String>) -> Vec<KnownVuln> {
    let mut out = Vec::new();
    for (comp, result) in components.iter().zip(osv) {
        for v in &result.vulns {
            let id = preferred_id(&v.id, &v.aliases);
            let kev_hit = std::iter::once(&id).chain(v.aliases.iter()).any(|a| kev.contains(a));
            out.push(KnownVuln {
                component: comp.name.clone(),
                version: comp.version.clone(),
                id,
                aliases: v.aliases.clone(),
                summary: v.summary.clone(),
                kev: kev_hit,
            });
        }
    }
    // KEV-flagged first, then by component.
    out.sort_by(|a, b| b.kev.cmp(&a.kev).then(a.component.cmp(&b.component)));
    out
}

/// Parse a CISA KEV JSON document into the set of exploited CVE ids.
#[must_use]
pub fn parse_kev(json: &str) -> BTreeSet<String> {
    #[derive(Deserialize)]
    struct Kev {
        #[serde(default)]
        vulnerabilities: Vec<KevEntry>,
    }
    #[derive(Deserialize)]
    struct KevEntry {
        #[serde(rename = "cveID")]
        cve_id: String,
    }
    serde_json::from_str::<Kev>(json)
        .map(|k| k.vulnerabilities.into_iter().map(|e| e.cve_id).collect())
        .unwrap_or_default()
}

/// Live OSV + KEV lookup. Network-bound; intended to run behind a CLI flag.
///
/// # Errors
/// Propagates HTTP/client construction errors. A failed KEV fetch is non-fatal
/// (vulns are still returned, just without exploited-in-the-wild flags).
pub async fn scan(components: &[Component]) -> Result<Vec<KnownVuln>, String> {
    if components.is_empty() {
        return Ok(Vec::new());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("js-beautify-rs/vuln-db")
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    // OSV batch query: one POST with all components.
    let queries: Vec<serde_json::Value> = components
        .iter()
        .map(|c| {
            serde_json::json!({
                "version": c.version,
                "package": { "name": c.name, "ecosystem": c.ecosystem },
            })
        })
        .collect();
    let body = serde_json::json!({ "queries": queries });
    let resp = client
        .post("https://api.osv.dev/v1/querybatch")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("osv request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("osv returned HTTP {}", resp.status()));
    }
    let osv: OsvBatchResponse = resp.json().await.map_err(|e| format!("osv decode: {e}"))?;

    // KEV is best-effort.
    let kev = match client
        .get("https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.text().await.ok().map(|t| parse_kev(&t)).unwrap_or_default(),
        _ => BTreeSet::new(),
    };

    Ok(merge_results(components, &osv.results, &kev))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osv_with(id: &str, aliases: &[&str], summary: &str) -> OsvResult {
        OsvResult {
            vulns: vec![OsvVuln {
                id: id.to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                summary: summary.to_string(),
            }],
        }
    }

    #[test]
    fn prefers_cve_alias_as_id() {
        assert_eq!(preferred_id("GHSA-abcd", &["CVE-2020-1234".into()]), "CVE-2020-1234");
        assert_eq!(preferred_id("GHSA-abcd", &[]), "GHSA-abcd");
    }

    #[test]
    fn merges_and_flags_kev() {
        let comps = vec![Component::npm("jquery", "1.7.0"), Component::npm("lodash", "4.17.10")];
        let osv = vec![
            osv_with("GHSA-jq", &["CVE-2020-11022"], "jQuery XSS"),
            osv_with("GHSA-lo", &["CVE-2019-10744"], "lodash prototype pollution"),
        ];
        let mut kev = BTreeSet::new();
        kev.insert("CVE-2019-10744".to_string());

        let vulns = merge_results(&comps, &osv, &kev);
        assert_eq!(vulns.len(), 2);
        // KEV-flagged sorts first.
        assert_eq!(vulns[0].id, "CVE-2019-10744");
        assert!(vulns[0].kev);
        assert_eq!(vulns[0].component, "lodash");
        assert!(!vulns[1].kev);
    }

    #[test]
    fn parses_kev_document() {
        let json = r#"{"vulnerabilities":[{"cveID":"CVE-2021-1"},{"cveID":"CVE-2021-2"}]}"#;
        let kev = parse_kev(json);
        assert_eq!(kev.len(), 2);
        assert!(kev.contains("CVE-2021-1"));
    }

    #[test]
    fn empty_osv_yields_no_vulns() {
        let comps = vec![Component::npm("safe-lib", "1.0.0")];
        let osv = vec![OsvResult::default()];
        assert!(merge_results(&comps, &osv, &BTreeSet::new()).is_empty());
    }
}
