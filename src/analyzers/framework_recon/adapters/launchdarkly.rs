use crate::analyzers::match_kind::{AnalyzerMatch, FileType, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::launchdarkly_extractor;

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("launchdarkly")
        || source.contains("ldclient")
        || source.contains("ldClient")
        || source.contains("LDClient")
        || source.contains("X-LaunchDarkly")
        || source.contains("variationDetail")
        || source.contains("boolVariation")
        || source.contains("stringVariation")
        || source.contains("numberVariation")
        || source.contains("jsonVariation"))
    {
        return Vec::new();
    }
    let result = launchdarkly_extractor::extract_feature_flags(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for flag in &result.flags {
        out.push(
            AnalyzerMatch::new(
                "launchdarkly_flag",
                flag.key.clone(),
                pos(source, &flag.key),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "default_value": flag.default_value,
                "ticket": flag.ticket,
                "deploy_date": flag.deploy_date,
            })),
        );
    }
    if let Some(key) = &result.config.sdk_key {
        out.push(
            AnalyzerMatch::new("launchdarkly_client_id", key.clone(), pos(source, key), file_type, fp())
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Medium),
        );
    }
    if let Some(env) = &result.config.environment {
        out.push(
            AnalyzerMatch::new(
                "launchdarkly_environment",
                env.clone(),
                pos(source, env),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchdarkly_adapter_runs_on_lowercase_ldclient_variation() {
        let source = r#"ldclient.variation("new-checkout-flow", false);"#;
        let matches = analyze(source, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "new-checkout-flow"),
            "expected LaunchDarkly flag from lowercase ldclient call: {matches:?}"
        );
    }

    #[test]
    fn launchdarkly_adapter_runs_on_camelcase_ldclient_variation() {
        let source = r#"ldClient.variation("new-checkout-flow", false);"#;
        let matches = analyze(source, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "new-checkout-flow"),
            "expected LaunchDarkly flag from camelCase ldClient call: {matches:?}"
        );
    }

    #[test]
    fn launchdarkly_adapter_runs_on_typed_variation_helper() {
        let source = r#"client.boolVariation("typed-flag", false);"#;
        let matches = analyze(source, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "typed-flag"),
            "expected LaunchDarkly flag from typed variation helper: {matches:?}"
        );
    }
}
