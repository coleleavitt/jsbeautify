//! Transmit Security SDK decoder — maps auth API calls and fingerprinting patterns.
//!
//! Schwab uses Transmit Security's XmSdk v10.15.7 for:
//! - Device fingerprinting (device_id, os_type, device_model)
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

use regex::Regex;

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
    pub has_webauthn: bool,
    pub has_cross_device: bool,
    pub has_debug_token: bool,
    pub risk_session_detected: bool,
    pub sdk_version: Option<String>,
}

/// Extract Transmit Security SDK patterns from source.
#[must_use]
pub fn analyze_transmit_sdk(source: &str) -> TransmitAnalysisResult {
    lazy_static::lazy_static! {
        static ref AUTH_ENDPOINT_RE: Regex = Regex::new(
            r#"["'](auth/[a-z_/]+)["']"#
        ).unwrap();

        static ref WEBAUTHN_RE: Regex = Regex::new(
            r#"["']((?:auth/)?webauthn/[a-z_/-]+)["']"#
        ).unwrap();

        static ref VERSION_RE: Regex = Regex::new(
            r#"(?:version|VERSION|sdk_version)['":\s]+["']?(\d+\.\d+\.\d+)"#
        ).unwrap();

        static ref FINGERPRINT_RE: Regex = Regex::new(
            r#"["'](device_id|device_model|os_type|os_version|persistence_mode|browser_name|browser_version|screen_resolution|timezone|language|platform)["']"#
        ).unwrap();

        static ref RISK_TOKEN_RE: Regex = Regex::new(
            r#"DRS-Session-Token|DRS-Action-Type|riskDataRequired|riskSessionToken"#
        ).unwrap();
    }

    let mut api_calls = Vec::new();
    let mut fingerprint_fields = Vec::new();

    // Extract auth endpoints
    for caps in AUTH_ENDPOINT_RE.captures_iter(source) {
        let endpoint = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let (purpose, security_note) = classify_auth_endpoint(&endpoint);
        api_calls.push(TransmitApiCall {
            endpoint,
            purpose,
            security_note,
        });
    }

    // Extract WebAuthn endpoints
    for caps in WEBAUTHN_RE.captures_iter(source) {
        let endpoint = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !api_calls.iter().any(|c| c.endpoint == endpoint) {
            api_calls.push(TransmitApiCall {
                endpoint,
                purpose: "WebAuthn/FIDO2 operation".to_string(),
                security_note: None,
            });
        }
    }

    // Extract fingerprint fields
    for caps in FINGERPRINT_RE.captures_iter(source) {
        let field = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let category = classify_fingerprint_field(&field);
        if !fingerprint_fields
            .iter()
            .any(|f: &FingerprintField| f.field_name == field)
        {
            fingerprint_fields.push(FingerprintField {
                field_name: field,
                category,
            });
        }
    }

    // Detect SDK version
    let sdk_version = VERSION_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let has_webauthn = source.contains("webauthn/authenticate") || source.contains("webauthn/register");
    let has_cross_device = source.contains("cross-device") || source.contains("cross_device");
    let has_debug_token = source.contains("create_debug_token");
    let risk_session_detected = RISK_TOKEN_RE.is_match(source);

    // Deduplicate
    api_calls.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
    api_calls.dedup_by(|a, b| a.endpoint == b.endpoint);

    TransmitAnalysisResult {
        api_calls,
        fingerprint_fields,
        has_webauthn,
        has_cross_device,
        has_debug_token,
        risk_session_detected,
        sdk_version,
    }
}

fn classify_auth_endpoint(endpoint: &str) -> (String, Option<String>) {
    match endpoint {
        "auth/login" => ("Initial authentication".into(), None),
        "auth/logout" => ("Session termination".into(), None),
        "auth/poll" => ("Async authentication status polling".into(), None),
        "auth/bind" => ("Device-to-session binding".into(), None),
        "auth/assert" => ("Authentication assertion verification".into(), None),
        "auth/sso" => ("Single sign-on federation".into(), None),
        "auth/key_exchange" => (
            "Cryptographic key exchange (DH/ECDH)".into(),
            Some("Key material exchanged — verify forward secrecy".into()),
        ),
        "auth/anonymous_invoke" => (
            "Unauthenticated policy invocation".into(),
            Some("Can be called without auth — test for info disclosure".into()),
        ),
        "auth/authenticator/config_menu" => ("MFA method configuration listing".into(), None),
        "auth/authenticator/register" => (
            "New authenticator enrollment".into(),
            Some("Test for enrollment without verification".into()),
        ),
        "auth/authenticator/unregister" => (
            "Authenticator removal".into(),
            Some("Test for unauthenticated removal".into()),
        ),
        "auth/approve" => ("Transaction/action approval".into(), None),
        "auth/create_debug_token" => (
            "Debug token creation".into(),
            Some("CRITICAL: debug endpoint in production code!".into()),
        ),
        e if e.contains("webauthn/authenticate") => ("WebAuthn login (FIDO2 assertion)".into(), None),
        e if e.contains("webauthn/register") => ("WebAuthn credential creation (FIDO2 attestation)".into(), None),
        e if e.contains("cross-device") || e.contains("cross_device") => (
            "Cross-device authentication flow".into(),
            Some("Ticket-based — test for ticket enumeration".into()),
        ),
        _ => (format!("Unknown Transmit endpoint: {endpoint}"), None),
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
        output.push_str(&format!("// ║ SDK Version: {ver}\n"));
    }

    output.push_str(&format!(
        "// ║ WebAuthn/FIDO2: {}\n",
        if result.has_webauthn { "YES" } else { "NO" }
    ));
    output.push_str(&format!(
        "// ║ Cross-Device Auth: {}\n",
        if result.has_cross_device { "YES" } else { "NO" }
    ));
    output.push_str(&format!(
        "// ║ Risk Scoring (DRS): {}\n",
        if result.risk_session_detected { "YES" } else { "NO" }
    ));

    if result.has_debug_token {
        output.push_str("// ║ ⚠️  DEBUG TOKEN ENDPOINT DETECTED IN PRODUCTION!\n");
    }

    if !result.api_calls.is_empty() {
        output.push_str("// ║\n// ║ AUTH ENDPOINTS:\n");
        for call in &result.api_calls {
            output.push_str(&format!("// ║   {} — {}\n", call.endpoint, call.purpose));
            if let Some(ref note) = call.security_note {
                output.push_str(&format!("// ║     ⚠️  {note}\n"));
            }
        }
    }

    if !result.fingerprint_fields.is_empty() {
        output.push_str("// ║\n// ║ DEVICE FINGERPRINT FIELDS:\n");
        for fp in &result.fingerprint_fields {
            output.push_str(&format!("// ║   {} ({})\n", fp.field_name, fp.category));
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
        assert!(result.has_webauthn);
        assert!(result.has_debug_token);
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
        assert!(result.risk_session_detected);
    }
}
