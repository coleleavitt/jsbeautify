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

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
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
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
    /// CVSS base score (0.0–10.0) parsed from the OSV `severity` vector, if present.
    pub cvss: Option<f64>,
    /// CVSS vector string (e.g. `CVSS:3.1/AV:N/...`), if present.
    pub cvss_vector: Option<String>,
    /// EPSS exploit-probability (0.0–1.0) for the CVE, if available.
    pub epss: Option<f64>,
}

// ── OSV response model (only the fields we use) ──────────────────────────────

/// One component's OSV result (the vulns affecting it).
///
/// Matches the `/v1/query` response body and each element of a `/v1/querybatch`
/// result. Public so the pure [`merge_results`] core can be exercised by tests
/// and callers without the network.
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
    /// OSV `severity` array: `[{type: "CVSS_V3", score: "CVSS:3.1/..."}]`.
    #[serde(default)]
    severity: Vec<OsvSeverity>,
}
#[derive(Debug, Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    score: String,
}

/// Compute the CVSS v3.1 **base score** from a `CVSS:3.x/AV:.../...` vector.
///
/// Implements the published CVSS v3.1 base-score formula (deterministic, so it's
/// unit-tested). Returns `None` for a non-v3 vector or one missing base metrics;
/// the raw vector string is surfaced separately for display either way.
#[must_use]
pub fn cvss_v3_base_score(vector: &str) -> Option<f64> {
    if !vector.starts_with("CVSS:3") {
        return None;
    }
    let mut m = std::collections::HashMap::new();
    for part in vector.split('/').skip(1) {
        if let Some((k, v)) = part.split_once(':') {
            m.insert(k, v);
        }
    }
    let av = match *m.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.2,
        _ => return None,
    };
    let ac = match *m.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let ui = match *m.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let scope_changed = *m.get("S")? == "C";
    // Privileges Required depends on scope.
    let pr = match (*m.get("PR")?, scope_changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.5,
        _ => return None,
    };
    let imp = |s: &str| match s {
        "H" => 0.56,
        "L" => 0.22,
        "N" => 0.0,
        _ => f64::NAN,
    };
    let (c, i, a) = (imp(m.get("C")?), imp(m.get("I")?), imp(m.get("A")?));
    if c.is_nan() || i.is_nan() || a.is_nan() {
        return None;
    }

    let iscbase = ((1.0 - c) * (1.0 - i)).mul_add(-(1.0 - a), 1.0);
    let impact = if scope_changed {
        7.52f64.mul_add(iscbase - 0.029, -3.25 * (iscbase - 0.02).powi(15))
    } else {
        6.42 * iscbase
    };
    let exploitability = 8.22 * av * ac * pr * ui;
    if impact <= 0.0 {
        return Some(0.0);
    }
    let raw = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };
    // CVSS rounds up to one decimal place.
    Some((raw * 10.0).ceil() / 10.0)
}

/// Best CVSS (score, vector) from an OSV vuln's severity array (prefer V3).
fn cvss_of(v: &OsvVuln) -> (Option<f64>, Option<String>) {
    let pick = v
        .severity
        .iter()
        .find(|s| s.kind.starts_with("CVSS_V3"))
        .or_else(|| v.severity.first());
    match pick {
        Some(s) if !s.score.is_empty() => (cvss_v3_base_score(&s.score), Some(s.score.clone())),
        _ => (None, None),
    }
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
/// Pure: this is the testable core. Each `OsvResult` is a `/v1/query` response
/// (or one `/v1/querybatch` result), already deserialized. If `osv` is shorter
/// than `components` the surplus components simply contribute nothing — but the
/// live [`scan`] always passes a vector of exactly `components.len()` (one entry
/// per query, empty on failure), so no finding is silently dropped.
#[must_use]
pub fn merge_results(components: &[Component], osv: &[OsvResult], kev: &BTreeSet<String>) -> Vec<KnownVuln> {
    debug_assert!(
        osv.len() == components.len() || osv.len() <= components.len(),
        "osv results should be aligned 1:1 with components",
    );
    let mut out = Vec::new();
    for (comp, result) in components.iter().zip(osv) {
        for v in &result.vulns {
            let id = preferred_id(&v.id, &v.aliases);
            // KEV can list the CVE *or* the GHSA id, so check the raw OSV id and
            // the preferred id and every alias — not just the preferred id.
            let kev_hit = std::iter::once(v.id.as_str())
                .chain(std::iter::once(id.as_str()))
                .chain(v.aliases.iter().map(String::as_str))
                .any(|a| kev.contains(a));
            let (cvss, cvss_vector) = cvss_of(v);
            out.push(KnownVuln {
                component: comp.name.clone(),
                version: comp.version.clone(),
                id,
                aliases: v.aliases.clone(),
                summary: v.summary.clone(),
                kev: kev_hit,
                cvss,
                cvss_vector,
                epss: None, // filled in by enrich_epss after the EPSS fetch
            });
        }
    }
    // KEV first, then highest CVSS, then component, then id (stable).
    out.sort_by(|a, b| {
        b.kev
            .cmp(&a.kev)
            .then(b.cvss.unwrap_or(0.0).total_cmp(&a.cvss.unwrap_or(0.0)))
            .then(a.component.cmp(&b.component))
            .then(a.id.cmp(&b.id))
    });
    out
}

/// Parse an EPSS API response (`api.first.org/data/v1/epss`) into a
/// `CVE -> probability` map. Pure + testable.
#[must_use]
pub fn parse_epss(json: &str) -> HashMap<String, f64> {
    #[derive(Deserialize)]
    struct Epss {
        #[serde(default)]
        data: Vec<EpssEntry>,
    }
    #[derive(Deserialize)]
    struct EpssEntry {
        cve: String,
        epss: String,
    }
    serde_json::from_str::<Epss>(json)
        .map(|e| {
            e.data
                .into_iter()
                .filter_map(|d| d.epss.parse::<f64>().ok().map(|p| (d.cve, p)))
                .collect()
        })
        .unwrap_or_default()
}

/// Attach EPSS scores to vulns whose id is a CVE present in `epss`.
pub fn enrich_epss<S: std::hash::BuildHasher>(vulns: &mut [KnownVuln], epss: &HashMap<String, f64, S>) {
    for v in vulns.iter_mut() {
        // The CVE id may be the preferred id or hide among the aliases.
        let cve = std::iter::once(&v.id)
            .chain(v.aliases.iter())
            .find(|a| a.starts_with("CVE-"));
        if let Some(p) = cve.and_then(|c| epss.get(c)) {
            v.epss = Some(*p);
        }
    }
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
/// Uses OSV's per-component `/v1/query` endpoint rather than `/v1/querybatch`:
/// the batch endpoint returns only `{id, modified}` per vuln (no aliases, no
/// summary), which would make CVE-id preference and KEV matching non-functional.
/// `/v1/query` returns the full vulnerability records. Queries run concurrently
/// and the result vector is kept aligned 1:1 with `components` (a failed query
/// yields an empty result, never a dropped component).
///
/// # Errors
/// Propagates HTTP-client construction errors. Per-component query failures and
/// a failed KEV fetch are non-fatal (that component just contributes no vulns /
/// no exploited-in-the-wild flags).
pub async fn scan(components: &[Component]) -> Result<Vec<KnownVuln>, String> {
    if components.is_empty() {
        return Ok(Vec::new());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("js-beautify-rs/vuln-db")
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    // One /v1/query per component, concurrently. Order is preserved so the
    // results stay 1:1 with `components`.
    let results: Vec<OsvResult> = stream::iter(components.iter())
        .map(|c| {
            let client = client.clone();
            async move { query_one(&client, c).await }
        })
        .buffered(8)
        .collect()
        .await;

    // KEV is best-effort.
    let kev = match client
        .get("https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.text().await.ok().map(|t| parse_kev(&t)).unwrap_or_default(),
        _ => BTreeSet::new(),
    };

    let mut vulns = merge_results(components, &results, &kev);

    // EPSS enrichment (best-effort): one request for all matched CVE ids.
    let cve_ids: Vec<String> = vulns
        .iter()
        .flat_map(|v| std::iter::once(&v.id).chain(v.aliases.iter()))
        .filter(|a| a.starts_with("CVE-"))
        .cloned()
        .collect();
    if !cve_ids.is_empty() {
        let mut deduped: Vec<String> = cve_ids;
        deduped.sort();
        deduped.dedup();
        let url = format!("https://api.first.org/data/v1/epss?cve={}", deduped.join(","));
        if let Ok(r) = client.get(&url).send().await
            && r.status().is_success()
            && let Ok(body) = r.text().await
        {
            enrich_epss(&mut vulns, &parse_epss(&body));
        }
    }

    Ok(vulns)
}

/// Query OSV `/v1/query` for one component, returning its full vuln records.
/// Any failure yields an empty result so the caller's 1:1 alignment is kept.
async fn query_one(client: &reqwest::Client, c: &Component) -> OsvResult {
    let body = serde_json::json!({
        "version": c.version,
        "package": { "name": c.name, "ecosystem": c.ecosystem },
    });
    match client.post("https://api.osv.dev/v1/query").json(&body).send().await {
        Ok(resp) if resp.status().is_success() => resp.json::<OsvResult>().await.unwrap_or_default(),
        _ => OsvResult::default(),
    }
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
                severity: Vec::new(),
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
    fn flags_kev_when_only_ghsa_id_listed() {
        // KEV lists the GHSA id, not the CVE alias — must still flag.
        let comps = vec![Component::npm("jquery", "1.7.0")];
        let osv = vec![osv_with("GHSA-jq-111", &["CVE-2020-11022"], "jQuery XSS")];
        let mut kev = BTreeSet::new();
        kev.insert("GHSA-jq-111".to_string());
        let vulns = merge_results(&comps, &osv, &kev);
        assert_eq!(vulns.len(), 1);
        assert!(vulns[0].kev, "raw GHSA id present in KEV must set kev=true");
        // Display id still prefers the CVE alias.
        assert_eq!(vulns[0].id, "CVE-2020-11022");
    }

    #[test]
    fn surplus_components_without_results_are_safe() {
        // Fewer OSV results than components must not panic or misalign.
        let comps = vec![Component::npm("a", "1.0.0"), Component::npm("b", "2.0.0")];
        let osv = vec![osv_with("GHSA-a", &[], "a vuln")]; // only one result
        let vulns = merge_results(&comps, &osv, &BTreeSet::new());
        assert_eq!(vulns.len(), 1);
        assert_eq!(vulns[0].component, "a");
    }

    #[test]
    fn parses_kev_document() {
        let json = r#"{"vulnerabilities":[{"cveID":"CVE-2021-1"},{"cveID":"CVE-2021-2"}]}"#;
        let kev = parse_kev(json);
        assert_eq!(kev.len(), 2);
        assert!(kev.contains("CVE-2021-1"));
    }

    #[test]
    fn computes_cvss_v3_base_scores() {
        // Known reference vectors → published base scores.
        // CVE-2014-0160 (Heartbleed) example vector → 7.5.
        assert_eq!(
            cvss_v3_base_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N"),
            Some(7.5)
        );
        // Full critical (all High, scope unchanged) → 9.8.
        assert_eq!(
            cvss_v3_base_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            Some(9.8)
        );
        // Scope-changed XSS-style → 6.1.
        assert_eq!(
            cvss_v3_base_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N"),
            Some(6.1)
        );
        // Non-v3 vector → None.
        assert_eq!(cvss_v3_base_score("CVSS:2.0/AV:N/AC:L/Au:N/C:P/I:P/A:P"), None);
    }

    #[test]
    fn parses_epss_and_enriches() {
        let json = r#"{"status":"OK","data":[
            {"cve":"CVE-2020-11022","epss":"0.00432","percentile":"0.7"},
            {"cve":"CVE-2019-10744","epss":"0.97","percentile":"0.99"}
        ]}"#;
        let epss = parse_epss(json);
        assert!((epss["CVE-2020-11022"] - 0.00432).abs() < 1e-9);

        let mut vulns = vec![
            KnownVuln {
                component: "jquery".into(),
                version: "1.7.0".into(),
                id: "CVE-2020-11022".into(),
                aliases: vec![],
                summary: String::new(),
                kev: false,
                cvss: Some(6.1),
                cvss_vector: None,
                epss: None,
            },
            KnownVuln {
                component: "x".into(),
                version: "1".into(),
                id: "GHSA-only".into(),
                aliases: vec!["CVE-2019-10744".into()],
                summary: String::new(),
                kev: false,
                cvss: None,
                cvss_vector: None,
                epss: None,
            },
        ];
        enrich_epss(&mut vulns, &epss);
        assert_eq!(vulns[0].epss, Some(0.00432));
        // EPSS found via an alias CVE.
        assert_eq!(vulns[1].epss, Some(0.97));
    }

    #[test]
    fn merge_extracts_cvss_from_severity() {
        let comps = vec![Component::npm("jquery", "1.7.0")];
        let osv = vec![OsvResult {
            vulns: vec![OsvVuln {
                id: "GHSA-x".into(),
                aliases: vec!["CVE-2020-11022".into()],
                summary: "XSS".into(),
                severity: vec![OsvSeverity {
                    kind: "CVSS_V3".into(),
                    score: "CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N".into(),
                }],
            }],
        }];
        let vulns = merge_results(&comps, &osv, &BTreeSet::new());
        assert_eq!(vulns[0].cvss, Some(6.1));
        assert!(vulns[0].cvss_vector.as_deref().unwrap().starts_with("CVSS:3.1"));
    }

    #[test]
    fn empty_osv_yields_no_vulns() {
        let comps = vec![Component::npm("safe-lib", "1.0.0")];
        let osv = vec![OsvResult::default()];
        assert!(merge_results(&comps, &osv, &BTreeSet::new()).is_empty());
    }
}
