//! Transmit Security SDK decoder — maps auth API calls and fingerprinting patterns.
//!
//! Schwab uses Transmit Security's `XmSdk` v10.15.7 for:
//! - Device fingerprinting (`device_id`, `os_type`, `device_model`)
//! - WebAuthn/FIDO2 registration and authentication
//! - Risk scoring (DRS-Session-Token)
//! - Multi-factor authentication orchestration
//! - Cross-device authentication flows
//!
//! API endpoints and their purposes:
//! | Endpoint | Purpose |
//! |----------|---------|
//! | `auth/login` | Initial authentication |
//! | `auth/poll` | Async auth status polling |
//! | `auth/bind` | Device binding |
//! | `auth/assert` | Assertion verification |
//! | `auth/sso` | Single sign-on flow |
//! | `auth/key_exchange` | Cryptographic key exchange |
//! | `auth/anonymous_invoke` | Unauthenticated policy invocation |
//! | `auth/authenticator/config_menu` | MFA method configuration |
//! | `auth/authenticator/register` | New authenticator registration |
//! | `auth/webauthn/authenticate/start` | WebAuthn login initiation |
//! | `auth/webauthn/register/start` | WebAuthn credential creation |
//! | `auth/webauthn/cross-device/*` | Cross-device FIDO2 flows |
//! | `auth/approve` | Transaction approval |
//! | `auth/create_debug_token` | Debug/test token (SHOULD NOT BE IN PROD) |

use crate::recon_budget::ReconBudget;
use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

/// A detected Transmit Security API call.
#[derive(Debug, Clone)]
pub struct TransmitApiCall {
    pub endpoint: String,
    pub purpose: String,
    pub security_note: Option<String>,
}

/// A detected device fingerprint field.
#[derive(Debug, Clone)]
pub struct FingerprintField {
    pub field_name: String,
    pub category: &'static str,
}

/// Result of Transmit SDK analysis.
#[derive(Debug)]
pub struct TransmitAnalysisResult {
    pub api_calls: Vec<TransmitApiCall>,
    pub fingerprint_fields: Vec<FingerprintField>,
    pub capabilities: TransmitCapabilities,
    pub sdk_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransmitCapabilities(u8);

impl TransmitCapabilities {
    const WEBAUTHN: u8 = 1;
    const CROSS_DEVICE: u8 = 1 << 1;
    const DEBUG_TOKEN: u8 = 1 << 2;
    const RISK_SESSION: u8 = 1 << 3;

    const fn insert(&mut self, flag: u8) {
        self.0 |= flag;
    }

    #[must_use]
    pub const fn has_webauthn(self) -> bool {
        self.0 & Self::WEBAUTHN != 0
    }

    #[must_use]
    pub const fn has_cross_device(self) -> bool {
        self.0 & Self::CROSS_DEVICE != 0
    }

    #[must_use]
    pub const fn has_debug_token(self) -> bool {
        self.0 & Self::DEBUG_TOKEN != 0
    }

    #[must_use]
    pub const fn risk_session_detected(self) -> bool {
        self.0 & Self::RISK_SESSION != 0
    }
}

/// Extract Transmit Security SDK patterns from source.
#[must_use]
pub fn analyze_transmit_sdk(source: &str) -> TransmitAnalysisResult {
    analyze_transmit_sdk_with_limit(source, usize::MAX)
}

/// Analyze while retaining at most `max_findings` in each result collection.
#[must_use]
pub fn analyze_transmit_sdk_with_limit(source: &str, max_findings: usize) -> TransmitAnalysisResult {
    analyze_transmit_sdk_with_budget(source, max_findings, &mut ReconBudget::new(usize::MAX))
}

#[must_use]
pub fn analyze_transmit_sdk_with_budget(
    source: &str,
    max_findings: usize,
    budget: &mut ReconBudget,
) -> TransmitAnalysisResult {
    static AUTH_ENDPOINT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["'](auth/[a-z_/]+)["']"#).unwrap());

    static WEBAUTHN_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["']((?:auth/)?webauthn/[a-z_/-]+)["']"#).unwrap());

    static VERSION_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?:version|VERSION|sdk_version)['":\s]+["']?(\d+\.\d+\.\d+)"#).unwrap());

    static FINGERPRINT_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"["'](device_id|device_model|os_type|os_version|persistence_mode|browser_name|browser_version|screen_resolution|timezone|language|platform)["']"#
        ).unwrap()
    });

    static RISK_TOKEN_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"DRS-Session-Token|DRS-Action-Type|riskDataRequired|riskSessionToken").unwrap());

    let mut api_calls = Vec::new();
    let mut fingerprint_fields = Vec::new();

    // Extract auth endpoints
    for caps in AUTH_ENDPOINT_RE.captures_iter(source) {
        if api_calls.len() >= max_findings {
            break;
        }
        let endpoint = caps.get(1).map_or("", |matched| matched.as_str());
        let (purpose, security_note) = classify_auth_endpoint(endpoint);
        let Some(endpoint) = budget.take_str(endpoint) else {
            break;
        };
        let Some(purpose) = budget.take_str(purpose) else {
            break;
        };
        let security_note = match security_note {
            Some(note) => {
                let Some(note) = budget.take_str(note) else {
                    break;
                };
                Some(note)
            }
            None => None,
        };
        api_calls.push(TransmitApiCall {
            endpoint,
            purpose,
            security_note,
        });
    }

    // Extract WebAuthn endpoints
    for caps in WEBAUTHN_RE.captures_iter(source) {
        if api_calls.len() >= max_findings {
            break;
        }
        let endpoint = caps.get(1).map_or("", |matched| matched.as_str());
        if !api_calls.iter().any(|call| call.endpoint == endpoint) {
            let Some(endpoint) = budget.take_str(endpoint) else {
                break;
            };
            let Some(purpose) = budget.take_str("WebAuthn/FIDO2 operation") else {
                break;
            };
            api_calls.push(TransmitApiCall {
                endpoint,
                purpose,
                security_note: None,
            });
        }
    }

    // Extract fingerprint fields
    for caps in FINGERPRINT_RE.captures_iter(source) {
        if fingerprint_fields.len() >= max_findings {
            break;
        }
        let field = caps.get(1).map_or("", |matched| matched.as_str());
        let category = classify_fingerprint_field(field);
        if !fingerprint_fields
            .iter()
            .any(|fingerprint: &FingerprintField| fingerprint.field_name == field)
        {
            let Some(field_name) = budget.take_str(field) else {
                break;
            };
            fingerprint_fields.push(FingerprintField { field_name, category });
        }
    }

    // Detect SDK version
    let sdk_version = VERSION_RE
        .captures(source)
        .and_then(|captures| captures.get(1))
        .and_then(|matched| budget.take_str(matched.as_str()));

    let mut capabilities = TransmitCapabilities::default();
    if source.contains("webauthn/authenticate") || source.contains("webauthn/register") {
        capabilities.insert(TransmitCapabilities::WEBAUTHN);
    }
    if source.contains("cross-device") || source.contains("cross_device") {
        capabilities.insert(TransmitCapabilities::CROSS_DEVICE);
    }
    if source.contains("create_debug_token") {
        capabilities.insert(TransmitCapabilities::DEBUG_TOKEN);
    }
    if RISK_TOKEN_RE.is_match(source) {
        capabilities.insert(TransmitCapabilities::RISK_SESSION);
    }

    // Deduplicate
    api_calls.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
    api_calls.dedup_by(|a, b| a.endpoint == b.endpoint);

    TransmitAnalysisResult {
        api_calls,
        fingerprint_fields,
        capabilities,
        sdk_version,
    }
}

fn classify_auth_endpoint(endpoint: &str) -> (&'static str, Option<&'static str>) {
    match endpoint {
        "auth/login" => ("Initial authentication", None),
        "auth/logout" => ("Session termination", None),
        "auth/poll" => ("Async authentication status polling", None),
        "auth/bind" => ("Device-to-session binding", None),
        "auth/assert" => ("Authentication assertion verification", None),
        "auth/sso" => ("Single sign-on federation", None),
        "auth/key_exchange" => (
            "Cryptographic key exchange (DH/ECDH)",
            Some("Key material exchanged — verify forward secrecy"),
        ),
        "auth/anonymous_invoke" => (
            "Unauthenticated policy invocation",
            Some("Can be called without auth — test for info disclosure"),
        ),
        "auth/authenticator/config_menu" => ("MFA method configuration listing", None),
        "auth/authenticator/register" => (
            "New authenticator enrollment",
            Some("Test for enrollment without verification"),
        ),
        "auth/authenticator/unregister" => ("Authenticator removal", Some("Test for unauthenticated removal")),
        "auth/approve" => ("Transaction/action approval", None),
        "auth/create_debug_token" => (
            "Debug token creation",
            Some("CRITICAL: debug endpoint in production code!"),
        ),
        e if e.contains("webauthn/authenticate") => ("WebAuthn login (FIDO2 assertion)", None),
        e if e.contains("webauthn/register") => ("WebAuthn credential creation (FIDO2 attestation)", None),
        e if e.contains("cross-device") || e.contains("cross_device") => (
            "Cross-device authentication flow",
            Some("Ticket-based — test for ticket enumeration"),
        ),
        _ => ("Unknown Transmit endpoint", None),
    }
}

fn classify_fingerprint_field(field: &str) -> &'static str {
    match field {
        "device_id" | "device_model" | "persistence_mode" => "device_identity",
        "os_type" | "os_version" | "platform" => "platform_info",
        "browser_name" | "browser_version" => "browser_info",
        "screen_resolution" | "timezone" | "language" => "environment",
        _ => "unknown",
    }
}

/// Format the analysis result as an annotation block.
#[must_use]
pub fn format_transmit_analysis(result: &TransmitAnalysisResult) -> String {
    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║         TRANSMIT SECURITY SDK ANALYSIS                  ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");

    if let Some(ref ver) = result.sdk_version {
        let _ = writeln!(output, "// ║ SDK Version: {ver}");
    }

    let _ = writeln!(
        output,
        "// ║ WebAuthn/FIDO2: {}",
        if result.capabilities.has_webauthn() {
            "YES"
        } else {
            "NO"
        }
    );
    let _ = writeln!(
        output,
        "// ║ Cross-Device Auth: {}",
        if result.capabilities.has_cross_device() {
            "YES"
        } else {
            "NO"
        }
    );
    let _ = writeln!(
        output,
        "// ║ Risk Scoring (DRS): {}",
        if result.capabilities.risk_session_detected() {
            "YES"
        } else {
            "NO"
        }
    );

    if result.capabilities.has_debug_token() {
        output.push_str("// ║ ⚠️  DEBUG TOKEN ENDPOINT DETECTED IN PRODUCTION!\n");
    }

    if !result.api_calls.is_empty() {
        output.push_str("// ║\n// ║ AUTH ENDPOINTS:\n");
        for call in &result.api_calls {
            let _ = writeln!(output, "// ║   {} — {}", call.endpoint, call.purpose);
            if let Some(ref note) = call.security_note {
                let _ = writeln!(output, "// ║     ⚠️  {note}");
            }
        }
    }

    if !result.fingerprint_fields.is_empty() {
        output.push_str("// ║\n// ║ DEVICE FINGERPRINT FIELDS:\n");
        for fp in &result.fingerprint_fields {
            let _ = writeln!(output, "// ║   {} ({})", fp.field_name, fp.category);
        }
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_auth_endpoints() {
        let source = r#"
            fetch(baseUrl + "auth/login", opts);
            fetch(baseUrl + "auth/webauthn/authenticate/start", opts);
            const url = "auth/create_debug_token";
        "#;

        let result = analyze_transmit_sdk(source);
        assert!(result.api_calls.iter().any(|c| c.endpoint == "auth/login"));
        assert!(result.capabilities.has_webauthn());
        assert!(result.capabilities.has_debug_token());
    }

    #[test]
    fn test_detect_fingerprinting() {
        let source = r#"
            payload["device_id"] = getDeviceId();
            payload["os_type"] = navigator.platform;
            payload["device_model"] = getModel();
        "#;

        let result = analyze_transmit_sdk(source);
        assert!(result.fingerprint_fields.len() >= 3);
    }

    #[test]
    fn test_risk_session() {
        let source = r#"headers["DRS-Session-Token"] = riskToken; headers["DRS-Action-Type"] = "login";"#;
        let result = analyze_transmit_sdk(source);
        assert!(result.capabilities.risk_session_detected());
    }
}
