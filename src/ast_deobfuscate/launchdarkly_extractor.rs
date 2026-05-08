//! LaunchDarkly feature flag extractor — identifies all feature flags and their defaults.
//!
//! Schwab uses LaunchDarkly for feature gating. Flag names follow the pattern:
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

/// LaunchDarkly SDK configuration.
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

/// Extract LaunchDarkly feature flag patterns from source.
#[must_use]
pub fn extract_feature_flags(source: &str) -> FeatureFlagResult {
    lazy_static::lazy_static! {
        // variation("FlagName", defaultValue)
        static ref VARIATION_RE: Regex = Regex::new(
            r#"variation\(\s*"([^"]+)"(?:\s*,\s*([^)]+))?\)"#
        ).unwrap();

        // SDK key patterns (LaunchDarkly client IDs are typically 24-char hex)
        static ref SDK_KEY_RE: Regex = Regex::new(
            r#"["']([0-9a-f]{24})["']"#
        ).unwrap();

        // Flag key used in other contexts
        static ref FLAG_KEY_RE: Regex = Regex::new(
            r#"["'](RelTgl_[A-Za-z0-9_-]+)["']"#
        ).unwrap();

        // LaunchDarkly identify call
        static ref IDENTIFY_RE: Regex = Regex::new(
            r#"(?:ldclient|LDClient)\.identify\("#
        ).unwrap();

        // Ticket/date pattern from flag name
        static ref TICKET_RE: Regex = Regex::new(
            r"(?:EMOST|Emost)-(\d{6})"
        ).unwrap();

        static ref DATE_RE: Regex = Regex::new(
            r"_(\d{8})$"
        ).unwrap();
    }

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

    // Also catch flags referenced outside variation() calls
    for caps in FLAG_KEY_RE.captures_iter(source) {
        let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
        if !flags.iter().any(|f| f.key == key) {
            let ticket = TICKET_RE
                .captures(&key)
                .and_then(|c| c.get(1))
                .map(|m| format!("EMOST-{}", m.as_str()));
            let deploy_date = DATE_RE
                .captures(&key)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string());

            flags.push(FeatureFlag {
                key,
                default_value: None,
                ticket,
                deploy_date,
            });
        }
    }

    // Look for SDK key (24-char hex near LaunchDarkly references)
    let sdk_key = if source.contains("LaunchDarkly") || source.contains("ldclient") {
        SDK_KEY_RE
            .captures(source)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
    } else {
        None
    };

    let config = LdConfig {
        sdk_key,
        environment: None,
    };

    flags.sort_by(|a, b| a.key.cmp(&b.key));

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
    output.push_str(&format!(
        "// ║ Total flags: {}  |  Evaluations: {}\n",
        result.flags.len(),
        result.total_evaluations
    ));

    if let Some(ref key) = result.config.sdk_key {
        output.push_str(&format!("// ║ SDK Key: {key}\n"));
    }

    output.push_str("// ║\n// ║ FLAGS:\n");
    for flag in &result.flags {
        let default = flag.default_value.as_deref().unwrap_or("?");
        let ticket = flag.ticket.as_deref().unwrap_or("");
        output.push_str(&format!("// ║   {} (default: {}) {}\n", flag.key, default, ticket));
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
    fn test_extract_flag_keys_outside_variation() {
        let source = r#"
            const flag = "RelTgl_EMOST-306801_A2_20230627";
            if (this.featureService.isEnabled(flag)) { ... }
        "#;

        let result = extract_feature_flags(source);
        assert_eq!(result.flags.len(), 1);
        assert_eq!(result.flags[0].ticket.as_deref(), Some("EMOST-306801"));
    }
}
