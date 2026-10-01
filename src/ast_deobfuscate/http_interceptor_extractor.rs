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

use crate::recon_budget::ReconBudget;
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
    analyze_interceptor_chain_with_limit(source, usize::MAX)
}

/// Analyze while retaining at most `max_findings` in each result collection.
#[must_use]
pub fn analyze_interceptor_chain_with_limit(source: &str, max_findings: usize) -> InterceptorAnalysisResult {
    analyze_interceptor_chain_with_budget(source, max_findings, &mut ReconBudget::new(usize::MAX))
}

#[must_use]
pub fn analyze_interceptor_chain_with_budget(
    source: &str,
    max_findings: usize,
    budget: &mut ReconBudget,
) -> InterceptorAnalysisResult {
    let mut headers = Vec::new();
    let mut token_storage = Vec::new();
    let mut policy_bypass_flags = Vec::new();

    // Extract header assignments
    for caps in HEADER_ASSIGN_RE.captures_iter(source).take(max_findings) {
        let header_match = caps.get(1).map_or("", |m| m.as_str());
        let value_expr = caps.get(2).map_or("", |m| m.as_str()).trim();
        let Some(header) = budget.take_str(header_match) else {
            break;
        };
        let Some(value_source) = classify_value_source(value_expr, budget) else {
            break;
        };
        let Ok(security_note) = classify_header_security(&header, budget) else {
            break;
        };

        headers.push(HeaderInjection {
            header,
            value_source,
            condition: None,
            security_note,
        });
    }

    // Extract sessionStorage tokens
    for caps in SESSION_STORAGE_RE.captures_iter(source) {
        if token_storage.len() >= max_findings {
            break;
        }
        let key_match = caps.get(1).map_or("", |m| m.as_str());
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key_match) {
            let Some(token_name) = budget.take_str(key_match) else {
                break;
            };
            let Some(key) = budget.take_str(key_match) else {
                break;
            };
            token_storage.push(TokenStorage {
                token_name,
                storage_type: "sessionStorage",
                key,
            });
        }
    }

    // Extract localStorage tokens
    for caps in LOCAL_STORAGE_RE.captures_iter(source) {
        if token_storage.len() >= max_findings {
            break;
        }
        let key_match = caps.get(1).map_or("", |m| m.as_str());
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key_match) {
            let Some(token_name) = budget.take_str(key_match) else {
                break;
            };
            let Some(key) = budget.take_str(key_match) else {
                break;
            };
            token_storage.push(TokenStorage {
                token_name,
                storage_type: "localStorage",
                key,
            });
        }
    }

    // Extract cookie-based tokens
    for caps in COOKIE_RE.captures_iter(source) {
        if token_storage.len() >= max_findings {
            break;
        }
        let key_match = caps.get(1).map_or("", |m| m.as_str());
        if !token_storage.iter().any(|t: &TokenStorage| t.key == key_match) {
            let Some(token_name) = budget.take_str(key_match) else {
                break;
            };
            let Some(key) = budget.take_str(key_match) else {
                break;
            };
            token_storage.push(TokenStorage {
                token_name,
                storage_type: "cookie",
                key,
            });
        }
    }

    // Extract policy bypass flags
    for caps in POLICY_BYPASS_RE.captures_iter(source) {
        if policy_bypass_flags.len() >= max_findings {
            break;
        }
        let flag_match = caps.get(1).map_or("", |m| m.as_str());
        if !policy_bypass_flags.iter().any(|flag| flag == flag_match) {
            let Some(flag) = budget.take_str(flag_match) else {
                break;
            };
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

fn classify_value_source(expr: &str, budget: &mut ReconBudget) -> Option<String> {
    if expr.contains("sessionStorage") {
        budget.take_str("sessionStorage")
    } else if expr.contains("localStorage") {
        budget.take_str("localStorage")
    } else if expr.contains("cookie") {
        budget.take_str("cookie")
    } else if expr.contains("this.") {
        let property = expr.split('.').nth(1).unwrap_or("?");
        let prefix = budget.take_str("instance property (")?;
        let property = budget.take_str(property)?;
        let suffix = budget.take_str(")")?;
        Some(format!("{prefix}{property}{suffix}"))
    } else if expr.starts_with('"') || expr.starts_with('\'') {
        let prefix = budget.take_str("literal: ")?;
        let expr = budget.take_str(expr)?;
        Some(format!("{prefix}{expr}"))
    } else {
        let prefix = budget.take_str("variable: ")?;
        let value: &str = &expr[..expr.char_indices().nth(40).map_or(expr.len(), |(index, _)| index)];
        let value = budget.take_str(value)?;
        Some(format!("{prefix}{value}"))
    }
}

fn classify_header_security(header: &str, budget: &mut ReconBudget) -> Result<Option<String>, ()> {
    let note = match header {
        "Schwab-Skip-Policy-Zero" => Some("POLICY BYPASS: skips initial auth policy — test if spoofable"),
        "Schwab-Skip-Status" => Some("STATUS BYPASS: skips status check — test if spoofable"),
        "Schwab-Skip-Level-Zero" => Some("LEVEL BYPASS: skips level-zero policy — test if spoofable"),
        "X-Authorization" => Some("Secondary auth token — test for token replay"),
        "DRS-Session-Token" => Some("Risk engine session — test for session fixation"),
        "swscrypto-signature" => Some("Web Crypto signature — verify server validates"),
        "swscrypto-public-key" => Some("Client public key — test for key substitution"),
        "web-crypto-enabled" => Some("Feature flag header — test if disabling weakens auth"),
        "Schwab-Passkey-Login" => Some("Passkey flow indicator — test for flow confusion"),
        h if h.contains("Migration") => Some("Migration flag — may enable legacy (weaker) paths"),
        _ => None,
    };
    note.map_or(Ok(None), |note| budget.take_str(note).map(Some).ok_or(()))
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
