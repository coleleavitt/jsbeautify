//! Attack-chain correlation: link individual findings into multi-step chains.
//!
//! A single finding is a lead. Real impact comes from *chaining*: a leaked token
//! plus an IDOR endpoint plus an open redirect is an account-takeover path, even
//! though no one of those is critical alone. This module correlates the flat
//! finding list our analyzers produce into [`AttackChain`]s — ordered sequences
//! of findings connected by a shared artifact (host, endpoint, or data) and a
//! known escalation pattern — and scores each chain by severity + length.
//!
//! Deterministic and pure (no LLM, no network): the agent tier layers reasoning
//! on top, but the structural correlation is rule-based and unit-tested. Chains
//! feed both the report and the live-validation queue (validate the *chain*, not
//! just isolated findings).
#![allow(clippy::doc_markdown)] // prose names tools/CWE ids/brands

use serde::Serialize;

use crate::analyzers::match_kind::AnalyzerMatch;

/// One correlated multi-step attack path.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttackChain {
    /// Stable kind, e.g. `"account_takeover"`, `"secret_to_api"`.
    pub kind: String,
    /// Human summary of the escalation.
    pub title: String,
    /// Ordered steps (indices into the source finding list) with a role label.
    pub steps: Vec<ChainStep>,
    /// 0–100 score: blends step severities + chain length + KEV/validated flags.
    pub score: u32,
    /// CWE of the *outcome* (the chain's worst impact).
    pub cwe: &'static str,
}

/// One step in a chain: which finding plays which role.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChainStep {
    /// Index into the finding list passed to [`correlate`].
    pub finding_index: usize,
    /// What this step contributes, e.g. `"credential"`, `"sink"`, `"pivot"`.
    pub role: &'static str,
    pub match_kind: String,
    pub match_value: String,
}

/// Severity → points.
const fn sev_points(m: &AnalyzerMatch) -> u32 {
    use crate::analyzers::regex_bank::Severity;
    match m.severity {
        Severity::Critical => 40,
        Severity::High => 30,
        Severity::Medium => 18,
        Severity::Low => 8,
        Severity::Info => 2,
    }
}

/// True if the finding is flagged exploited-in-the-wild (KEV) in metadata.
fn is_kev(m: &AnalyzerMatch) -> bool {
    m.metadata
        .as_ref()
        .and_then(|v| v.get("kev"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// The host a finding pertains to (from file_path URL), for cross-finding joins.
fn finding_host(m: &AnalyzerMatch) -> Option<String> {
    let fp = m.file_path.as_deref()?;
    let after = fp.split("://").nth(1).unwrap_or(fp);
    let host = after.split(['/', ':', '?']).next()?;
    host.contains('.').then(|| host.to_ascii_lowercase())
}

fn step(findings: &[AnalyzerMatch], i: usize, role: &'static str) -> ChainStep {
    ChainStep {
        finding_index: i,
        role,
        match_kind: findings[i].match_kind.clone(),
        match_value: findings[i].match_value.clone(),
    }
}

/// Indices of findings whose kind is in `kinds`.
fn indices_of<'a>(findings: &'a [AnalyzerMatch], kinds: &'a [&str]) -> impl Iterator<Item = usize> + 'a {
    findings
        .iter()
        .enumerate()
        .filter(move |(_, m)| kinds.contains(&m.match_kind.as_str()))
        .map(|(i, _)| i)
}

/// Correlate findings into attack chains. Findings on the same host strengthen a
/// chain; chains are scored and returned highest-first.
#[must_use]
pub fn correlate(findings: &[AnalyzerMatch]) -> Vec<AttackChain> {
    let mut chains = Vec::new();

    let secrets: Vec<usize> = indices_of(
        findings,
        &["high_entropy_secret", "google_api_key", "aws_access_key", "github_pat"],
    )
    .collect();
    let idors: Vec<usize> = indices_of(findings, &["idor_candidate"]).collect();
    let redirects: Vec<usize> = indices_of(findings, &["open_redirect"]).collect();
    let xss: Vec<usize> = indices_of(findings, &["dom_xss"]).collect();
    let cves: Vec<usize> = indices_of(findings, &["known_cve"]).collect();

    // Chain 1: open redirect + a token/secret on the same host → account takeover
    // (steal the token via the redirect).
    for &r in &redirects {
        let rhost = finding_host(&findings[r]);
        for &s in &secrets {
            if rhost.is_none() || finding_host(&findings[s]) == rhost {
                let base = sev_points(&findings[r]) + sev_points(&findings[s]);
                chains.push(AttackChain {
                    kind: "account_takeover".to_string(),
                    title: "Open redirect can exfiltrate a token/credential exposed on the same origin".to_string(),
                    steps: vec![step(findings, s, "credential"), step(findings, r, "exfil-vector")],
                    score: (base + 20).min(100),
                    cwe: "CWE-601",
                });
            }
        }
    }

    // Chain 2: DOM-XSS + IDOR endpoint → authenticated action via injected script.
    for &x in &xss {
        for &d in &idors {
            let base = sev_points(&findings[x]) + sev_points(&findings[d]);
            chains.push(AttackChain {
                kind: "xss_to_idor".to_string(),
                title: "DOM-XSS can drive the IDOR endpoint with the victim's session".to_string(),
                steps: vec![step(findings, x, "injection"), step(findings, d, "pivot")],
                score: (base + 15).min(100),
                cwe: "CWE-79",
            });
        }
    }

    // Chain 3: known CVE (esp. KEV) + a reachable sink/endpoint → exploit delivery.
    for &c in &cves {
        let kev_bonus = if is_kev(&findings[c]) { 25 } else { 0 };
        for &x in xss.iter().chain(idors.iter()) {
            let base = sev_points(&findings[c]) + sev_points(&findings[x]);
            chains.push(AttackChain {
                kind: "known_cve_reachable".to_string(),
                title: "A component with a known CVE sits alongside a reachable sink/endpoint".to_string(),
                steps: vec![
                    step(findings, c, "vulnerable-component"),
                    step(findings, x, "reachability"),
                ],
                score: (base + kev_bonus).min(100),
                cwe: "CWE-1035",
            });
        }
    }

    // Chain 4: secret + IDOR → direct authenticated object access with the leaked key.
    for &s in &secrets {
        let shost = finding_host(&findings[s]);
        for &d in &idors {
            if shost.is_none() || finding_host(&findings[d]) == shost {
                let base = sev_points(&findings[s]) + sev_points(&findings[d]);
                chains.push(AttackChain {
                    kind: "secret_to_idor".to_string(),
                    title: "Leaked credential + object-reference endpoint enables direct data access".to_string(),
                    steps: vec![step(findings, s, "credential"), step(findings, d, "object-access")],
                    score: (base + 18).min(100),
                    cwe: "CWE-639",
                });
            }
        }
    }

    chains.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.kind.cmp(&b.kind)));
    chains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::match_kind::{FileType, PositionRange, Provenance};
    use crate::analyzers::regex_bank::Severity;

    fn mk(kind: &str, value: &str, sev: Severity, host: Option<&str>) -> AnalyzerMatch {
        AnalyzerMatch::new(
            kind,
            value,
            PositionRange::new(1, 0, 1, 1),
            FileType::Js,
            host.map(|h| format!("https://{h}/app.js")),
        )
        .with_provenance(Provenance::Ast)
        .with_severity(sev)
    }

    #[test]
    fn chains_open_redirect_with_secret_on_same_host() {
        let findings = vec![
            mk(
                "open_redirect",
                "location.href=u",
                Severity::Medium,
                Some("app.target.com"),
            ),
            mk(
                "high_entropy_secret",
                "aB3xZ9qL...",
                Severity::High,
                Some("app.target.com"),
            ),
        ];
        let chains = correlate(&findings);
        assert!(chains.iter().any(|c| c.kind == "account_takeover"), "{chains:?}");
        assert!(chains[0].score > 0);
    }

    #[test]
    fn chains_xss_with_idor() {
        let findings = vec![
            mk("dom_xss", "el.innerHTML=u", Severity::High, Some("t.com")),
            mk("idor_candidate", "/accounts/123", Severity::Low, Some("t.com")),
        ];
        let chains = correlate(&findings);
        assert!(chains.iter().any(|c| c.kind == "xss_to_idor"));
    }

    #[test]
    fn kev_cve_scores_higher() {
        let mut kev_cve = mk("known_cve", "jquery@1.7.0 CVE-x", Severity::Critical, Some("t.com"));
        kev_cve.metadata = Some(serde_json::json!({ "kev": true }));
        let findings = vec![kev_cve, mk("idor_candidate", "/users/1", Severity::Low, Some("t.com"))];
        let chains = correlate(&findings);
        let cve_chain = chains.iter().find(|c| c.kind == "known_cve_reachable").unwrap();
        assert!(cve_chain.score >= 60, "KEV should boost score: {}", cve_chain.score);
    }

    #[test]
    fn no_chains_from_unrelated_findings() {
        let findings = vec![mk("hostname", "a.com", Severity::Info, None)];
        assert!(correlate(&findings).is_empty());
    }

    #[test]
    fn cross_host_secret_and_redirect_still_chains_when_host_unknown() {
        // file_path absent → host unknown → we don't over-filter.
        let findings = vec![
            mk("open_redirect", "x", Severity::Medium, None),
            mk("high_entropy_secret", "y", Severity::High, None),
        ];
        assert!(correlate(&findings).iter().any(|c| c.kind == "account_takeover"));
    }
}
