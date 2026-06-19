//! Built-in secret/credential detection regexes.
//!
//! Patterns from jxscout-pro-v2 v2.1.1 + extractify + trufflehog + gitleaks.
//! 60+ literal patterns covering all major cloud providers and services.

mod literal;
mod macros;
mod property;
mod types;

pub use types::{SecretRule, Severity};

/// All built-in rules.
#[must_use]
pub fn all_rules() -> Vec<SecretRule> {
    let mut rules = Vec::new();
    literal::append_rules(&mut rules);
    property::append_rules(&mut rules);
    rules
}

/// Only the literal-value rules (non-property patterns).
#[must_use]
pub fn literal_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| !r.is_property_name).collect()
}

/// Only the property-name rules.
#[must_use]
pub fn property_name_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| r.is_property_name).collect()
}

/// Get severity for a given `match_kind` (falls back to Info for unknown kinds).
#[must_use]
pub fn severity_for_kind(kind: &str) -> Severity {
    for rule in all_rules() {
        if rule.kind == kind {
            return rule.severity;
        }
    }
    // Analyzer-specific defaults
    match kind {
        "html_manipulation" | "dangerously_set_inner_html" | "dom_xss" => Severity::High,
        // Taint-flow + entropy findings (severity usually set explicitly).
        "open_redirect" | "high_entropy_secret" | "location_assignment" | "window_onmessage" | "post_message" => {
            Severity::Medium
        }
        "url_search_params" | "storage_access" | "graphql" | "api_call" => Severity::Low,
        "path" | "hostname" | "url" | "comment_path" | "comment_hostname" | "npm_package" | "package_json_dep" => {
            Severity::Info
        }
        "package_json" => Severity::Medium,
        _ if kind.starts_with("sensitive_property_name:") => Severity::Medium,
        _ if kind.starts_with("custom:") => Severity::Medium,
        _ => Severity::Info,
    }
}
