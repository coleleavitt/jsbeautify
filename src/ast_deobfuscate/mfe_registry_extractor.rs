//! SAC Shell MFE registry extractor — maps micro-frontend loading paths.
//!
//! Schwab's Advisor Center uses single-spa for micro-frontend orchestration:
//! 1. Shell loads `/config` → receives gateway URLs
//! 2. MFEs registered via single-spa, activated by hash route
//! 3. MFE HTML fetched via XHR (NO SRI/integrity checks)
//! 4. Scripts dynamically injected into DOM from parsed HTML
//!
//! Security concerns:
//! - No Subresource Integrity (SRI) on dynamically-loaded MFE scripts
//! - MFE HTML fetched over XHR without integrity verification
//! - Cross-origin script injection from CDN without pinning
//! - Shared cookie/token scope between all MFEs

use crate::recon_budget::ReconBudget;
use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

/// A detected micro-frontend registration.
#[derive(Debug, Clone)]
pub struct MfeRegistration {
    /// MFE name (e.g. "sac-dashboard-ui")
    pub name: String,
    /// Activation route/hash pattern
    pub route_pattern: Option<String>,
    /// Source URL pattern for loading
    pub source_url: Option<String>,
    /// Whether SRI/integrity is present
    pub has_integrity_check: bool,
}

/// A detected dynamic script injection.
#[derive(Debug, Clone)]
pub struct ScriptInjection {
    /// How the script URL is constructed
    pub url_pattern: String,
    /// Whether nonce/integrity is applied
    pub has_security_attrs: bool,
    /// The injection method (createElement, innerHTML, etc.)
    pub method: String,
    /// Byte offset of the injection site in the source (for positioning).
    pub offset: usize,
}

/// Result of MFE registry analysis.
#[derive(Debug)]
pub struct MfeAnalysisResult {
    pub registrations: Vec<MfeRegistration>,
    pub script_injections: Vec<ScriptInjection>,
    pub config_endpoints: Vec<String>,
    pub has_sri: bool,
    pub has_csp_nonce: bool,
    pub shared_auth_scope: bool,
}

/// Extract MFE registry patterns from source.
#[must_use]
pub fn analyze_mfe_registry(source: &str) -> MfeAnalysisResult {
    analyze_mfe_registry_with_limit(source, usize::MAX)
}

/// Analyze while retaining at most `max_findings` in each result collection.
#[must_use]
pub fn analyze_mfe_registry_with_limit(source: &str, max_findings: usize) -> MfeAnalysisResult {
    analyze_mfe_registry_with_budget(source, max_findings, &mut ReconBudget::new(usize::MAX))
}

#[must_use]
pub fn analyze_mfe_registry_with_budget(
    source: &str,
    max_findings: usize,
    budget: &mut ReconBudget,
) -> MfeAnalysisResult {
    // single-spa registerApplication
    static REGISTER_APP_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"registerApplication\(\s*["']([^"']+)["']"#).unwrap());

    // MFE name patterns (sac-*-ui, as-*-ui)
    static MFE_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"["']((?:sac|as|money-movement|import-transactions|acd|managed-accounts)-[\w-]+-ui)["']"#).unwrap()
    });

    // Dynamic script creation
    static SCRIPT_CREATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"createElement\(\s*["']script["']\)"#).unwrap());

    // Config endpoint
    static CONFIG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["'](/config|/mfe-config|/mfes/[^"']+)["']"#).unwrap());

    // Integrity attribute
    static INTEGRITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"integrity|crossorigin|sri").unwrap());

    // CSP nonce
    static NONCE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"nonce|\.nc\b").unwrap());

    let mut registrations = Vec::new();
    let mut script_injections = Vec::new();
    let mut config_endpoints = Vec::new();

    // Extract MFE registrations
    for caps in REGISTER_APP_RE.captures_iter(source).take(max_findings) {
        let Some(name) = caps.get(1).and_then(|matched| budget.take_str(matched.as_str())) else {
            break;
        };
        registrations.push(MfeRegistration {
            name,
            route_pattern: None,
            source_url: None,
            has_integrity_check: false,
        });
    }

    // Extract MFE names from string literals
    for caps in MFE_NAME_RE.captures_iter(source) {
        if registrations.len() >= max_findings {
            break;
        }
        let name_match = caps.get(1).map_or("", |m| m.as_str());
        if !registrations.iter().any(|r| r.name == name_match) {
            let Some(name) = budget.take_str(name_match) else {
                break;
            };
            registrations.push(MfeRegistration {
                name,
                route_pattern: None,
                source_url: None,
                has_integrity_check: false,
            });
        }
    }

    // Dynamic script injections — record each site's offset so callers can
    // position the finding on the actual `createElement("script")` rather than
    // a synthetic placeholder.
    for m in SCRIPT_CREATE_RE.find_iter(source).take(max_findings.min(10)) {
        let Some(url_pattern) = budget.take_str("dynamic") else {
            break;
        };
        let Some(method) = budget.take_str("createElement('script')") else {
            break;
        };
        script_injections.push(ScriptInjection {
            url_pattern,
            has_security_attrs: NONCE_RE.is_match(source),
            method,
            offset: m.start(),
        });
    }

    // Extract config endpoints
    for caps in CONFIG_RE.captures_iter(source) {
        if config_endpoints.len() >= max_findings {
            break;
        }
        let endpoint_match = caps.get(1).map_or("", |m| m.as_str());
        if !config_endpoints.iter().any(|endpoint| endpoint == endpoint_match) {
            let Some(endpoint) = budget.take_str(endpoint_match) else {
                break;
            };
            config_endpoints.push(endpoint);
        }
    }

    let has_sri = INTEGRITY_RE.is_match(source);
    let has_csp_nonce = NONCE_RE.is_match(source);
    // If multiple MFEs share the same auth cookies, it's shared scope
    let shared_auth_scope = registrations.len() > 1;

    MfeAnalysisResult {
        registrations,
        script_injections,
        config_endpoints,
        has_sri,
        has_csp_nonce,
        shared_auth_scope,
    }
}

/// Format as annotation block.
#[must_use]
pub fn format_mfe_analysis(result: &MfeAnalysisResult) -> String {
    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║      MICRO-FRONTEND (MFE) REGISTRY ANALYSIS            ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");
    let _ = writeln!(output, "// ║ MFEs detected: {}", result.registrations.len());
    let _ = writeln!(
        output,
        "// ║ Dynamic script injections: {}\n",
        result.script_injections.len()
    );
    let _ = writeln!(
        output,
        "// ║ SRI/Integrity checks: {}\n",
        if result.has_sri { "YES" } else { "⚠️  NO" }
    );
    let _ = writeln!(
        output,
        "// ║ CSP Nonce: {}\n",
        if result.has_csp_nonce { "YES" } else { "NO" }
    );
    let _ = writeln!(
        output,
        "// ║ Shared auth scope: {}\n",
        if result.shared_auth_scope {
            "⚠️  YES (all MFEs share tokens)"
        } else {
            "NO"
        }
    );

    if !result.registrations.is_empty() {
        output.push_str("// ║\n// ║ REGISTERED MFEs:\n");
        for mfe in &result.registrations {
            let _ = writeln!(output, "// ║   {}", mfe.name);
        }
    }

    if !result.has_sri {
        output.push_str("// ║\n// ║ ⚠️  SUPPLY CHAIN RISK: No SRI on dynamically-loaded scripts!\n");
        output.push_str("// ║     An attacker controlling the CDN/MFE server could inject\n");
        output.push_str("// ║     arbitrary JS into the advisor portal.\n");
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mfe_names() {
        let source = r#"
            registerApplication("sac-dashboard-ui", () => import("./dashboard"));
            const mfe = "sac-global-search-ui";
            loadMfe("as-alerts-ui");
        "#;

        let result = analyze_mfe_registry(source);
        assert!(result.registrations.iter().any(|r| r.name == "sac-dashboard-ui"));
        assert!(result.registrations.iter().any(|r| r.name == "sac-global-search-ui"));
        assert!(result.registrations.iter().any(|r| r.name == "as-alerts-ui"));
    }

    #[test]
    fn test_no_sri_detection() {
        let source = r#"
            var s = document.createElement("script");
            s.src = mfeUrl + "/main.js";
            document.head.appendChild(s);
        "#;

        let result = analyze_mfe_registry(source);
        assert!(!result.has_sri);
    }
}
