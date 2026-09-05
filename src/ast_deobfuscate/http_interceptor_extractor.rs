//! HTTP interceptor chain reconstructor — extracts Angular `HttpClient` interceptor logic.
//!
//! Angular apps inject headers via `HttpInterceptor` classes. In the Schwab SWS gateway,
//! the interceptor chain adds these security-critical headers:
//! - `Schwab-Client-SessionId` — session tracking
//! - `Schwab-Client-CorrelId` — request correlation
//! - `DRS-Session-Token` — Transmit Security risk token
//! - `DRS-Action-Type` — risk action classification
//! - `X-Authorization` — secondary auth token
//! - `Schwab-UUID` — user identifier
//! - `Schwab-Auth-Policy-Id` — auth policy routing
//! - `Schwab-Skip-Policy-Zero` — policy bypass flag
//! - `swscrypto-challenge` / `swscrypto-signature` / `swscrypto-public-key` — Web Crypto auth
//! - `Schwab-Client-Channel` / `Schwab-Client-Scope` — routing metadata
//! - `web-crypto-enabled` — feature flag for crypto auth

use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

// Header assignment: oe["Header-Name"] = value
static HEADER_ASSIGN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\w+\["(Schwab-[^"]+|DRS-[^"]+|X-Authorization|Authorization|web-crypto-enabled|swscrypto-[^"]+|tenant-config-service|Sch-Uid|Fraudnet-[^"]+|Use-SWS-[^"]+)"\]\s*=\s*([^;,\n]+)"#,
    )
    .unwrap()
});

// sessionStorage.getItem/setItem patterns
static SESSION_STORAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"sessionStorage\.(?:get|set)Item\("([^"]+)"\)"#).unwrap());

// localStorage patterns
static LOCAL_STORAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"localStorage\.(?:get|set)Item\("([^"]+)"\)"#).unwrap());

// Cookie reading patterns
static COOKIE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"document\.cookie[^;]*?["']([A-Z][A-Za-z0-9_-]+)=["']"#).unwrap());

// Policy bypass patterns
static POLICY_BYPASS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(Schwab-Skip-[^"]+|Schwab-Migration-[^"]+)""#).unwrap());

/// A detected HTTP header injection.
#[derive(Debug, Clone)]
pub struct HeaderInjection {
    /// The header name
    pub header: String,
    /// Where the value comes from (e.g. "sessionStorage", "this.correlationId", "cookie")
    pub value_source: String,
    /// Condition under which the header is added (if any)
    pub condition: Option<String>,
    /// Security relevance
    pub security_note: Option<String>,
}

/// A detected token storage location.
#[derive(Debug, Clone)]
pub struct TokenStorage {
    pub token_name: String,
    pub storage_type: &'static str, // "localStorage", "sessionStorage", "memory", "cookie", "header"
    pub key: String,
}

/// Result of interceptor chain analysis.
#[derive(Debug)]
pub struct InterceptorAnalysisResult {
    pub headers: Vec<HeaderInjection>,
    pub token_storage: Vec<TokenStorage>,
    pub has_crypto_auth: bool,
    pub has_risk_headers: bool,
    pub policy_bypass_flags: Vec<String>,
}

/// Extract HTTP interceptor patterns from beautified source.
#[must_use]
pub fn analyze_interceptor_chain(source: &str) -> InterceptorAnalysisResult {
    let mut headers = Vec::new();
    let mut token_storage = Vec::new();
    let mut policy_bypass_flags = Vec::new();

    // Extract header assignments
    for caps in HEADER_ASSIGN_RE.captures_iter(source) {
        let header = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let value_expr = caps.get(2).map_or("", |m| m.as_str()).trim().to_string();

        let value_source = classify_value_source(&value_expr);
        let security_note = classify_header_security(&header);

        headers.push(HeaderInjection {
            header,
            value_source,
            condition: None,
            security_note,
        });
    }

    // Extract sessionStorage tokens
    for caps in SESSION_STORAGE_RE.captures_iter(source) {
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key) {
            token_storage.push(TokenStorage {
                token_name: key.clone(),
                storage_type: "sessionStorage",
                key,
            });
        }
    }

    // Extract localStorage tokens
    for caps in LOCAL_STORAGE_RE.captures_iter(source) {
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key) {
            token_storage.push(TokenStorage {
                token_name: key.clone(),
                storage_type: "localStorage",
                key,
            });
        }
    }

    // Extract cookie-based tokens
    for caps in COOKIE_RE.captures_iter(source) {
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key) {
            token_storage.push(TokenStorage {
                token_name: key.clone(),
                storage_type: "cookie",
                key,
            });
        }
    }

    // Extract policy bypass flags
    for caps in POLICY_BYPASS_RE.captures_iter(source) {
        let flag = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !policy_bypass_flags.contains(&flag) {
            policy_bypass_flags.push(flag);
        }
    }

    // Deduplicate headers
    headers.sort_by(|a, b| a.header.cmp(&b.header));
    headers.dedup_by(|a, b| a.header == b.header);

    let has_crypto_auth = source.contains("swscrypto-") || source.contains("web-crypto-enabled");
    let has_risk_headers = source.contains("DRS-Session-Token") || source.contains("DRS-Action-Type");

    InterceptorAnalysisResult {
        headers,
        token_storage,
        has_crypto_auth,
        has_risk_headers,
        policy_bypass_flags,
    }
}

fn classify_value_source(expr: &str) -> String {
    if expr.contains("sessionStorage") {
        "sessionStorage".to_string()
    } else if expr.contains("localStorage") {
        "localStorage".to_string()
    } else if expr.contains("cookie") {
        "cookie".to_string()
    } else if expr.contains("this.") {
        format!("instance property ({})", expr.split('.').nth(1).unwrap_or("?"))
    } else if expr.starts_with('"') || expr.starts_with('\'') {
        format!("literal: {expr}")
    } else {
        format!("variable: {}", expr.chars().take(40).collect::<String>())
    }
}

fn classify_header_security(header: &str) -> Option<String> {
    match header {
        "Schwab-Skip-Policy-Zero" => Some("POLICY BYPASS: skips initial auth policy — test if spoofable".into()),
        "Schwab-Skip-Status" => Some("STATUS BYPASS: skips status check — test if spoofable".into()),
        "Schwab-Skip-Level-Zero" => Some("LEVEL BYPASS: skips level-zero policy — test if spoofable".into()),
        "X-Authorization" => Some("Secondary auth token — test for token replay".into()),
        "DRS-Session-Token" => Some("Risk engine session — test for session fixation".into()),
        "swscrypto-signature" => Some("Web Crypto signature — verify server validates".into()),
        "swscrypto-public-key" => Some("Client public key — test for key substitution".into()),
        "web-crypto-enabled" => Some("Feature flag header — test if disabling weakens auth".into()),
        "Schwab-Passkey-Login" => Some("Passkey flow indicator — test for flow confusion".into()),
        h if h.contains("Migration") => Some("Migration flag — may enable legacy (weaker) paths".into()),
        _ => None,
    }
}

/// Format as annotation block.
#[must_use]
pub fn format_interceptor_analysis(result: &InterceptorAnalysisResult) -> String {
    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║       HTTP INTERCEPTOR CHAIN ANALYSIS                   ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");
    let _ = writeln!(output, "// ║ Headers injected: {}", result.headers.len());
    let _ = writeln!(output, "// ║ Token storage locations: {}\n", result.token_storage.len());
    let _ = writeln!(
        output,
        "// ║ Web Crypto auth: {}\n",
        if result.has_crypto_auth { "YES" } else { "NO" }
    );
    let _ = writeln!(
        output,
        "// ║ Risk scoring headers: {}\n",
        if result.has_risk_headers { "YES" } else { "NO" }
    );

    if !result.policy_bypass_flags.is_empty() {
        output.push_str("// ║\n// ║ ⚠️  POLICY BYPASS FLAGS:\n");
        for flag in &result.policy_bypass_flags {
            let _ = writeln!(output, "// ║   {flag}");
        }
    }

    if !result.headers.is_empty() {
        output.push_str("// ║\n// ║ INJECTED HEADERS:\n");
        for h in &result.headers {
            let _ = writeln!(output, "// ║   {} ← {}", h.header, h.value_source);
            if let Some(ref note) = h.security_note {
                let _ = writeln!(output, "// ║     ⚠️  {note}");
            }
        }
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_headers() {
        let source = r#"
            oe["Schwab-Client-SessionId"] = this.sessionIdToken.sessionId;
            oe["DRS-Session-Token"] = Re;
            oe["Schwab-Skip-Policy-Zero"] = "true";
        "#;

        let result = analyze_interceptor_chain(source);
        assert!(result.headers.iter().any(|h| h.header == "Schwab-Client-SessionId"));
        assert!(result.has_risk_headers);
        assert!(
            result
                .policy_bypass_flags
                .iter()
                .any(|f| f.contains("Skip-Policy-Zero"))
        );
    }

    #[test]
    fn test_detect_storage() {
        let source = r#"
            sessionStorage.getItem("isRemembered");
            sessionStorage.setItem("alternate-experience", JSON.stringify(exp));
            localStorage.getItem("id_token");
        "#;

        let result = analyze_interceptor_chain(source);
        assert!(result.token_storage.iter().any(|t| t.key == "isRemembered"));
        assert!(result.token_storage.iter().any(|t| t.storage_type == "localStorage"));
    }
}
