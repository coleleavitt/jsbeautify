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
}
