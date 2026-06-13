//! Live validation: actively confirm a finding exists on the target.
//!
//! A static finding is a *hypothesis* ("this looks like an open redirect"). This
//! module turns hypotheses into evidence by interacting with the live target:
//!
//! - **open_redirect**: request the endpoint with an attacker URL in the
//!   redirect parameter; confirm the response `Location` actually points
//!   off-origin.
//! - **dom_xss / reflected**: send a unique canary in the candidate parameter;
//!   confirm it reflects unencoded in the response body.
//! - **idor_candidate**: request the object with two different ids; differing
//!   2xx bodies (without auth) suggest missing object-level authorization.
//! - **known_cve**: re-fetch the asset and confirm the vulnerable version string
//!   is still live.
//!
//! The **decision logic is pure and unit-tested** (`judge_*` functions take
//! already-fetched response facts); the HTTP layer is a thin wrapper behind a
//! flag so nothing touches the network in tests or by default. Output is a
//! [`Validation`] verdict attached to the finding.
#![allow(clippy::doc_markdown)] // prose names tools/CWE ids/brands

use serde::Serialize;

/// Verdict for one validation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Actively confirmed against the live target.
    Confirmed,
    /// Probed but not confirmed (likely false positive or protected).
    Refuted,
    /// Couldn't test (no network, unsupported kind, ambiguous).
    Inconclusive,
}

/// A validation result with supporting evidence.
#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    pub verdict: Verdict,
    /// Short human evidence string.
    pub evidence: String,
}

impl Validation {
    fn confirmed(evidence: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Confirmed,
            evidence: evidence.into(),
        }
    }
    fn refuted(evidence: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Refuted,
            evidence: evidence.into(),
        }
    }
    fn inconclusive(evidence: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Inconclusive,
            evidence: evidence.into(),
        }
    }
}

/// Decide an open-redirect verdict from the response to a probe that injected
/// `canary_host` into the redirect parameter.
///
/// `location` is the response `Location` header (if any); `status` the code.
#[must_use]
pub fn judge_open_redirect(status: u16, location: Option<&str>, canary_host: &str, origin_host: &str) -> Validation {
    let Some(loc) = location else {
        return Validation::refuted(format!("no Location header (HTTP {status})"));
    };
    let target_host = redirect_target_host(loc);
    match target_host {
        Some(h) if h.eq_ignore_ascii_case(canary_host) => {
            Validation::confirmed(format!("redirects off-origin to attacker host {h} (HTTP {status})"))
        }
        Some(h) if h.eq_ignore_ascii_case(origin_host) => {
            Validation::refuted(format!("redirect stays on-origin ({h})"))
        }
        Some(h) => Validation::inconclusive(format!("redirects to unexpected host {h}")),
        None => Validation::refuted(format!("Location is relative/on-site ({loc})")),
    }
}

/// The host a redirect `Location` points to, or `None` for relative redirects.
fn redirect_target_host(location: &str) -> Option<String> {
    let loc = location.trim();
    if let Some(rest) = loc.strip_prefix("//") {
        return rest.split(['/', '?', '#']).next().map(str::to_ascii_lowercase);
    }
    let after = loc.split("://").nth(1)?;
    after.split(['/', ':', '?', '#']).next().map(str::to_ascii_lowercase)
}

/// Decide a reflected-XSS verdict: did the unique `canary` appear unencoded in
/// the response body?
#[must_use]
pub fn judge_reflection(body: &str, canary: &str) -> Validation {
    if body.contains(canary) {
        Validation::confirmed(format!("canary {canary} reflected unencoded in response body"))
    } else {
        let encoded = canary.replace('<', "&lt;").replace('>', "&gt;");
        if body.contains(&encoded) {
            Validation::refuted("canary reflected but HTML-encoded (sanitized)")
        } else {
            Validation::refuted("canary not reflected")
        }
    }
}

/// Decide an IDOR verdict from two unauthenticated probes of different object
/// ids. Distinct successful bodies suggest object data is served without an
/// ownership check.
#[must_use]
pub fn judge_idor(status_a: u16, body_a: &str, status_b: u16, body_b: &str) -> Validation {
    let ok = |s: u16| (200..300).contains(&s);
    if ok(status_a) && ok(status_b) && body_a != body_b && !body_a.is_empty() && !body_b.is_empty() {
        Validation::confirmed("two object ids return distinct 2xx bodies without auth")
    } else if !ok(status_a) && !ok(status_b) {
        Validation::refuted(format!("both ids non-2xx ({status_a}/{status_b}) — likely auth-gated"))
    } else {
        Validation::inconclusive("responses inconclusive for object-level authz")
    }
}

/// Decide a known-CVE verdict: is the vulnerable `version` string still present
/// in the live asset body?
#[must_use]
pub fn judge_known_cve(asset_body: &str, version: &str) -> Validation {
    if version.is_empty() {
        return Validation::inconclusive("no version to confirm");
    }
    if asset_body.contains(version) {
        Validation::confirmed(format!("vulnerable version {version} present in live asset"))
    } else {
        Validation::refuted(format!("version {version} not found in live asset"))
    }
}

// ── Live prober ──────────────────────────────────────────────────────────────
//
// Wires the pure `judge_*` functions above to real HTTP. Two hard safety rails,
// both enforced before any request leaves the process:
//   1. **Scope**: only hosts that match the caller-supplied in-scope allowlist
//      ([`crate::scope::ScopeMatcher`]) are ever contacted. An out-of-scope URL
//      is an `Inconclusive` verdict, never a request.
//   2. **Rate limit**: a minimum delay is enforced between every request so the
//      prober can't hammer a target.
// This is the *authorized-target* counterpart to the static analyzers — it only
// confirms findings the tool already produced, against a host you explicitly
// scoped, behind the `--validate` flag.

use std::time::Duration;

use crate::scope::ScopeMatcher;

/// Configuration for the live prober.
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    /// Attacker-controlled host used as the open-redirect / SSRF canary target.
    pub canary_host: String,
    /// Minimum delay between requests (politeness / rate limit).
    pub min_delay: Duration,
    /// Per-request timeout.
    pub timeout: Duration,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            canary_host: "validate.example".to_string(),
            min_delay: Duration::from_millis(500),
            timeout: Duration::from_secs(15),
        }
    }
}

/// A scope- and rate-limited HTTP prober.
pub struct LiveProber {
    scope: ScopeMatcher,
    cfg: ProbeConfig,
    client: reqwest::Client,
}

impl LiveProber {
    /// Build a prober. `in_scope` is the allowlist of host patterns (e.g.
    /// `["*.target.com", "target.com"]`); nothing outside it is ever contacted.
    ///
    /// # Errors
    /// Fails if the HTTP client cannot be built.
    pub fn new(in_scope: &[&str], cfg: ProbeConfig) -> Result<Self, String> {
        let scope = ScopeMatcher::new(in_scope, &[]);
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            // Do NOT auto-follow redirects: we need to inspect the Location header.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("js-beautify-rs/validate")
            .build()
            .map_err(|e| format!("build prober client: {e}"))?;
        Ok(Self { scope, cfg, client })
    }

    /// Whether `url`'s host is in scope (the gate for any live request).
    #[must_use]
    pub fn in_scope(&self, url: &str) -> bool {
        host_of(url).is_some_and(|h| self.scope.is_in_scope(&h))
    }

    async fn rate_limit(&self) {
        tokio::time::sleep(self.cfg.min_delay).await;
    }

    /// Probe an open-redirect candidate: append the canary host to a redirect
    /// parameter and check whether the response redirects off-origin.
    ///
    /// # Errors
    /// Never returns `Err`; out-of-scope / transport failures become verdicts.
    pub async fn probe_open_redirect(&self, endpoint_url: &str) -> Validation {
        if !self.in_scope(endpoint_url) {
            return Validation::inconclusive(format!("out of scope: {endpoint_url}"));
        }
        let Some(origin_host) = host_of(endpoint_url) else {
            return Validation::inconclusive("could not parse endpoint host");
        };
        let canary = format!("https://{}/", self.cfg.canary_host);
        let sep = if endpoint_url.contains('?') { '&' } else { '?' };
        // Fuzz the common redirect param names in one shot is overkill here; use
        // the canonical one. Callers can pass an endpoint that already has the param.
        let probe_url = format!("{endpoint_url}{sep}redirect={canary}");

        self.rate_limit().await;
        match self.client.get(&probe_url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let location = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                judge_open_redirect(status, location.as_deref(), &self.cfg.canary_host, &origin_host)
            }
            Err(e) => Validation::inconclusive(format!("request failed: {e}")),
        }
    }

    /// Probe a reflected-input candidate: send a unique canary in the given
    /// query parameter and check whether it reflects unencoded.
    ///
    /// # Errors
    /// Never returns `Err`; failures become verdicts.
    pub async fn probe_reflection(&self, endpoint_url: &str, param: &str) -> Validation {
        if !self.in_scope(endpoint_url) {
            return Validation::inconclusive(format!("out of scope: {endpoint_url}"));
        }
        let canary = format!("xss{:x}probe", fastrand_seed());
        let sep = if endpoint_url.contains('?') { '&' } else { '?' };
        let probe_url = format!("{endpoint_url}{sep}{param}={canary}");

        self.rate_limit().await;
        match self.client.get(&probe_url).send().await {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                judge_reflection(&body, &canary)
            }
            Err(e) => Validation::inconclusive(format!("request failed: {e}")),
        }
    }

    /// Probe an IDOR candidate: fetch the original endpoint and a swapped-id
    /// variant, then compare. `swapped_url` must be produced by the caller (we
    /// don't guess the id position here).
    ///
    /// # Errors
    /// Never returns `Err`; failures become verdicts.
    pub async fn probe_idor(&self, original_url: &str, swapped_url: &str) -> Validation {
        if !self.in_scope(original_url) || !self.in_scope(swapped_url) {
            return Validation::inconclusive("out of scope");
        }
        self.rate_limit().await;
        let a = self.client.get(original_url).send().await;
        self.rate_limit().await;
        let b = self.client.get(swapped_url).send().await;
        match (a, b) {
            (Ok(ra), Ok(rb)) => {
                let sa = ra.status().as_u16();
                let sb = rb.status().as_u16();
                let ba = ra.text().await.unwrap_or_default();
                let bb = rb.text().await.unwrap_or_default();
                judge_idor(sa, &ba, sb, &bb)
            }
            _ => Validation::inconclusive("one or both IDOR probes failed"),
        }
    }

    /// Probe a known-CVE finding: re-fetch the asset and confirm the vulnerable
    /// version string is still served.
    ///
    /// # Errors
    /// Never returns `Err`; failures become verdicts.
    pub async fn probe_known_cve(&self, asset_url: &str, version: &str) -> Validation {
        if !self.in_scope(asset_url) {
            return Validation::inconclusive(format!("out of scope: {asset_url}"));
        }
        self.rate_limit().await;
        match self.client.get(asset_url).send().await {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                judge_known_cve(&body, version)
            }
            Err(e) => Validation::inconclusive(format!("request failed: {e}")),
        }
    }
}

/// Produce a swapped-id variant of a URL for IDOR probing: increment the first
/// all-numeric path segment (`/accounts/12345` → `/accounts/12346`). Returns
/// `None` when there's no numeric segment to swap.
#[must_use]
pub fn swap_first_id(url: &str) -> Option<String> {
    // Split off the query so we only touch the path.
    let (path_part, query) = url.split_once('?').map_or((url, None), |(p, q)| (p, Some(q)));
    let mut segments: Vec<String> = path_part.split('/').map(String::from).collect();
    let mut swapped = false;
    for seg in &mut segments {
        if !swapped
            && seg.len() >= 2
            && seg.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = seg.parse::<u64>()
        {
            *seg = n.wrapping_add(1).to_string();
            swapped = true;
        }
    }
    if !swapped {
        return None;
    }
    let rebuilt = segments.join("/");
    Some(match query {
        Some(q) => format!("{rebuilt}?{q}"),
        None => rebuilt,
    })
}

/// The lowercase host of a URL, or `None`.
fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1).unwrap_or(url);
    let host = after.split(['/', ':', '?', '#']).next()?;
    (!host.is_empty() && host.contains('.')).then(|| host.to_ascii_lowercase())
}

/// A cheap non-crypto seed for canary uniqueness (time-based). Good enough for a
/// reflection marker; not security-sensitive.
fn fastrand_seed() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_redirect_confirmed_when_offsite() {
        let v = judge_open_redirect(
            302,
            Some("https://evil.attacker.test/x"),
            "evil.attacker.test",
            "app.target.com",
        );
        assert_eq!(v.verdict, Verdict::Confirmed);
    }

    #[test]
    fn open_redirect_refuted_when_onsite_or_relative() {
        assert_eq!(
            judge_open_redirect(
                302,
                Some("https://app.target.com/home"),
                "evil.attacker.test",
                "app.target.com"
            )
            .verdict,
            Verdict::Refuted
        );
        assert_eq!(
            judge_open_redirect(302, Some("/dashboard"), "evil.attacker.test", "app.target.com").verdict,
            Verdict::Refuted
        );
        assert_eq!(
            judge_open_redirect(200, None, "evil.attacker.test", "app.target.com").verdict,
            Verdict::Refuted
        );
    }

    #[test]
    fn protocol_relative_redirect_is_detected() {
        let v = judge_open_redirect(
            302,
            Some("//evil.attacker.test/x"),
            "evil.attacker.test",
            "app.target.com",
        );
        assert_eq!(v.verdict, Verdict::Confirmed);
    }

    #[test]
    fn reflection_confirmed_only_when_unencoded() {
        let canary = "xss7f3qZ";
        assert_eq!(
            judge_reflection(&format!("<div>{canary}</div>"), canary).verdict,
            Verdict::Confirmed
        );
        assert_eq!(
            judge_reflection("<div>&lt;xss7f3qZ&gt;</div>", "<xss7f3qZ>").verdict,
            Verdict::Refuted
        );
        assert_eq!(judge_reflection("nothing here", canary).verdict, Verdict::Refuted);
    }

    #[test]
    fn idor_confirmed_on_distinct_bodies() {
        assert_eq!(
            judge_idor(200, "user A data", 200, "user B data").verdict,
            Verdict::Confirmed
        );
        assert_eq!(judge_idor(403, "", 403, "").verdict, Verdict::Refuted);
        assert_eq!(judge_idor(200, "same", 200, "same").verdict, Verdict::Inconclusive);
    }

    #[test]
    fn known_cve_confirmed_when_version_present() {
        assert_eq!(
            judge_known_cve("/*! jQuery v1.7.0 */ ...", "1.7.0").verdict,
            Verdict::Confirmed
        );
        assert_eq!(judge_known_cve("jQuery v3.7.1", "1.7.0").verdict, Verdict::Refuted);
    }

    #[test]
    fn prober_scope_gate_allows_in_scope_and_blocks_out_of_scope() {
        let prober = LiveProber::new(&["*.target.com", "target.com"], ProbeConfig::default()).unwrap();
        assert!(prober.in_scope("https://app.target.com/login"));
        assert!(prober.in_scope("https://target.com/x"));
        assert!(!prober.in_scope("https://evil.example/x"));
        assert!(!prober.in_scope("https://target.com.evil.example/x"));
    }

    #[tokio::test]
    async fn prober_refuses_out_of_scope_without_requesting() {
        // No network: an out-of-scope URL must short-circuit to Inconclusive.
        let prober = LiveProber::new(&["*.target.com"], ProbeConfig::default()).unwrap();
        let v = prober.probe_open_redirect("https://evil.example/go").await;
        assert_eq!(v.verdict, Verdict::Inconclusive);
        assert!(v.evidence.contains("out of scope"));

        let v2 = prober.probe_known_cve("https://evil.example/app.js", "1.0.0").await;
        assert_eq!(v2.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://a.b.com/x?y=1").as_deref(), Some("a.b.com"));
        assert_eq!(host_of("http://a.b.com:8080/x").as_deref(), Some("a.b.com"));
        assert!(host_of("https://localhost/x").is_none());
    }

    #[test]
    fn swaps_first_numeric_id_in_url() {
        assert_eq!(
            swap_first_id("https://t.com/api/accounts/12345").as_deref(),
            Some("https://t.com/api/accounts/12346")
        );
        assert_eq!(
            swap_first_id("https://t.com/users/42/orders?x=1").as_deref(),
            Some("https://t.com/users/43/orders?x=1")
        );
        assert!(swap_first_id("https://t.com/api/v2/users").is_none());
    }
}
