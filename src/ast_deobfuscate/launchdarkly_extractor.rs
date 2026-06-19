//! `LaunchDarkly` feature flag extractor — identifies all feature flags and their defaults.
//!
//! Schwab uses `LaunchDarkly` for feature gating. Flag names follow the pattern:
//! `RelTgl_EMOST-NNNNNN_Platform_Date` where:
//! - `RelTgl` = Release Toggle
//! - `EMOST-NNNNNN` = Jira ticket number
//! - Platform = A2 (Advisor 2.0), SWS2 (SWS Gateway), 3POS (Third Party)
//! - Date = deployment date (YYYYMMDD)
//!
//! Security relevance: feature flags often gate security controls (MFA, crypto auth,
//! new login flows). If flags can be influenced client-side, security features may be
//! bypassed.

use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

/// A detected feature flag evaluation.
#[derive(Debug, Clone)]
pub struct FeatureFlag {
    /// The flag key/name
    pub key: String,
    /// The default value (if visible)
    pub default_value: Option<String>,
    /// Platform/ticket info extracted from naming convention
    pub ticket: Option<String>,
    /// Deployment date from flag name
    pub deploy_date: Option<String>,
}

/// `LaunchDarkly` SDK configuration.
#[derive(Debug, Clone)]
pub struct LdConfig {
    /// Client-side SDK key
    pub sdk_key: Option<String>,
    /// Environment identifier
    pub environment: Option<String>,
}

/// Result of feature flag extraction.
#[derive(Debug)]
pub struct FeatureFlagResult {
    pub flags: Vec<FeatureFlag>,
    pub config: LdConfig,
    pub total_evaluations: usize,
}

/// Extract `LaunchDarkly` feature flag patterns from source.
#[must_use]
pub fn extract_feature_flags(source: &str) -> FeatureFlagResult {
    // Flag evaluation calls: variation/variationDetail and the typed
    // helpers (boolVariation/stringVariation/numberVariation/jsonVariation).
    // Group 1 = flag key, group 2 = default value (optional).
    static VARIATION_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?:variation|variationDetail|boolVariation|stringVariation|numberVariation|jsonVariation)\(\s*"([^"]+)"(?:\s*,\s*([^)]+?))?\)"#
        ).unwrap()
    });

    // SDK key patterns (LaunchDarkly client IDs are typically 24-char hex)
    static SDK_KEY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([0-9a-f]{24})["']"#).unwrap());

    // Client-side environment id embedded in LD stream/poll/event URLs:
    // /sdk/evalx/<envId>, /sdk/goals/<envId>, /eval/<envId>, /events/bulk/<envId>.
    static ENV_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/(?:sdk/(?:evalx|goals|eval)|eval|events/bulk)/([a-f0-9]{24})").unwrap());

    // LaunchDarkly identify call
    static IDENTIFY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:ldclient|LDClient)\.identify\(").unwrap());

    // Ticket/date pattern from flag name
    static TICKET_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:EMOST|Emost)-(\d{6})").unwrap());

    static DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(\d{8})$").unwrap());

    let mut flags = Vec::new();
    let total_evaluations = VARIATION_RE.find_iter(source).count();

    // Extract variation() calls
    for caps in VARIATION_RE.captures_iter(source) {
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let default_str = caps.get(2).map(|m| m.as_str().trim().to_string());

        let ticket = TICKET_RE
            .captures(&key)
            .and_then(|c| c.get(1))
            .map(|m| format!("EMOST-{}", m.as_str()));

        let deploy_date = DATE_RE
            .captures(&key)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        if !flags.iter().any(|f: &FeatureFlag| f.key == key) {
            flags.push(FeatureFlag {
                key,
                default_value: default_str,
                ticket,
                deploy_date,
            });
        }
    }

    // Definitive LD fingerprint: the unrenamed `X-LaunchDarkly-User-Agent`
    // header, plus the SDK identifiers / stream+event hosts.
    let is_launchdarkly = source.contains("X-LaunchDarkly")
        || source.contains("LaunchDarkly")
        || source.contains("LDClient")
        || IDENTIFY_RE.is_match(source)
        || source.contains("ldclient")
        || source.contains("ldClient")
        || source.contains("launchdarkly")
        || source.contains("clientstream.launchdarkly")
        || ENV_RE.is_match(source);

    // Look for SDK key (24-char hex near LaunchDarkly references)
    let sdk_key = if is_launchdarkly {
        SDK_KEY_RE
            .captures(source)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
    } else {
        None
    };

    // Client-side environment id from LD URLs (the most reliable concrete id).
    let environment = ENV_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let config = LdConfig { sdk_key, environment };

    flags.sort_by_key(|flag| flag.key.clone());

    FeatureFlagResult {
        flags,
        config,
        total_evaluations,
    }
}

/// Format as annotation block.
#[must_use]
pub fn format_feature_flags(result: &FeatureFlagResult) -> String {
    if result.flags.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║         LAUNCHDARKLY FEATURE FLAGS                      ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");
    let _ = writeln!(
        output,
        "// ║ Total flags: {}  |  Evaluations: {}\n",
        result.flags.len(),
        result.total_evaluations
    );

    if let Some(ref key) = result.config.sdk_key {
        let _ = writeln!(output, "// ║ SDK Key: {key}");
    }

    output.push_str("// ║\n// ║ FLAGS:\n");
    for flag in &result.flags {
        let default = flag.default_value.as_deref().unwrap_or("?");
        let ticket = flag.ticket.as_deref().unwrap_or("");
        let _ = writeln!(output, "// ║   {} (default: {}) {}", flag.key, default, ticket);
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_variation_calls() {
        let source = r#"
            this.ld.variation("RelTgl_EMOST-290345_A2_06062023", false);
            this.ld.variation("RelTgl_EMOST-440411_3POS_20250528", false);
            this.ld.variation("RelTgl_Emost-000000_Sws2_20220427", true);
        "#;

        let result = extract_feature_flags(source);
        assert_eq!(result.flags.len(), 3);
        assert_eq!(result.total_evaluations, 3);

        let flag = result.flags.iter().find(|f| f.key.contains("290345")).unwrap();
        assert_eq!(flag.ticket.as_deref(), Some("EMOST-290345"));
        assert_eq!(flag.deploy_date.as_deref(), Some("06062023"));
        assert_eq!(flag.default_value.as_deref(), Some("false"));
    }

    #[test]
    fn test_generic_typed_variations_and_environment() {
        // Generalized (de-Schwab): typed variation helpers + client-side env id
        // recovered from the LD streaming/eval URL.
        let source = r#"
            ldclient.boolVariation("new-checkout-flow", false);
            ldclient.stringVariation("pricing-tier", "control");
            fetch("https://clientstream.launchdarkly.com/eval/aabbccddeeff001122334455");
        "#;

        let result = extract_feature_flags(source);
        assert!(result.flags.iter().any(|f| f.key == "new-checkout-flow"));
        assert!(result.flags.iter().any(|f| f.key == "pricing-tier"));
        assert_eq!(result.config.environment.as_deref(), Some("aabbccddeeff001122334455"));
    }
}
