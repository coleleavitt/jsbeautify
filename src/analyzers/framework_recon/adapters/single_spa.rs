use crate::analyzers::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::mfe_registry_extractor;

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("registerApplication") || source.contains("singleSpa") || source.contains("single-spa")) {
        return Vec::new();
    }
    let result = mfe_registry_extractor::analyze_mfe_registry(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for app in &result.registrations {
        out.push(
            AnalyzerMatch::new(
                "single_spa_app",
                app.name.clone(),
                pos(source, &app.name),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "route_pattern": app.route_pattern,
                "source_url": app.source_url,
                "has_integrity_check": app.has_integrity_check,
                "has_sri": result.has_sri,
                "has_csp_nonce": result.has_csp_nonce,
                "shared_auth_scope": result.shared_auth_scope,
            })),
        );
    }
    for injection in &result.script_injections {
        let unverified = !injection.has_security_attrs && !result.has_sri;
        out.push(
            AnalyzerMatch::new(
                "single_spa_script_injection",
                injection.method.clone(),
                PositionRange::from_byte_range(source, injection.offset, injection.offset),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(if unverified { Severity::Medium } else { Severity::Info })
            .with_metadata(serde_json::json!({
                "method": injection.method,
                "url_pattern": injection.url_pattern,
                "has_security_attrs": injection.has_security_attrs,
                "has_sri": result.has_sri,
                "has_csp_nonce": result.has_csp_nonce,
            })),
        );
    }
    for endpoint in &result.config_endpoints {
        out.push(
            AnalyzerMatch::new(
                "single_spa_config_endpoint",
                endpoint.clone(),
                pos(source, endpoint),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low),
        );
    }
    out
}
