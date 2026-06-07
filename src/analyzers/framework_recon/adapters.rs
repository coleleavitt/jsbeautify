//! Adapters mapping the `ast_deobfuscate` framework extractors' result structs
//! into flat [`AnalyzerMatch`] findings for the recon pipeline.

use crate::analyzers::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::{
    angular_metadata, angular_route_extractor, http_interceptor_extractor, launchdarkly_extractor,
    mfe_registry_extractor, ngrx_extractor, rxjs_annotator,
};

/// Best-effort position: locate `needle` in `source`, else point at the start.
fn pos(source: &str, needle: &str) -> PositionRange {
    source.find(needle).map_or_else(
        || PositionRange::new(1, 0, 1, 0),
        |i| PositionRange::from_byte_range(source, i, i + needle.len()),
    )
}

/// NgRx: action catalog (the bracketed type strings survive minification),
/// effects, and store dispatches.
pub fn ngrx(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("createAction") || source.contains("createEffect") || source.contains(".dispatch(")) {
        return Vec::new();
    }
    let r = ngrx_extractor::extract_ngrx_patterns(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    for a in &r.actions {
        out.push(
            AnalyzerMatch::new(
                "ngrx_action",
                a.action_type.clone(),
                pos(source, &a.action_type),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "minified_name": a.minified_name,
                "has_props": a.has_props,
            })),
        );
    }
    for e in &r.effects {
        out.push(
            AnalyzerMatch::new(
                "ngrx_effect",
                e.trigger_actions.join(", "),
                PositionRange::new(1, 0, 1, 0),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "triggers": e.trigger_actions,
                "dispatches": e.dispatches,
            })),
        );
    }
    for d in &r.store_dispatches {
        out.push(
            AnalyzerMatch::new("ngrx_dispatch", d.clone(), pos(source, d), file_type, fp())
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info),
        );
    }
    out
}

/// Angular: component/selector registry + route table (recover component names,
/// inputs/outputs, lazy-route chunk graph).
pub fn angular(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    if source.contains("defineComponent")
        || source.contains("ɵcmp")
        || source.contains("ɵɵdefineComponent")
        || source.contains("definePipe")
        || source.contains("defineInjectable")
    {
        let meta = angular_metadata::extract_angular_metadata(source);
        for c in &meta.definitions {
            out.push(
                AnalyzerMatch::new(
                    "angular_component",
                    c.selector.clone(),
                    pos(source, &c.selector),
                    file_type,
                    fp(),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({
                    "class_name": c.class_name,
                    "kind": format!("{:?}", c.kind),
                    "inputs": c.inputs,
                    "outputs": c.outputs,
                    "minified_export_name": c.minified_export_name,
                    "minified_class_name": c.minified_class_name,
                })),
            );
        }
    }

    if source.contains("loadChildren")
        || source.contains("loadComponent")
        || source.contains("RouterModule")
        || source.contains("provideRouter")
    {
        let routes = angular_route_extractor::extract_routes(source);
        for r in &routes.routes {
            out.push(
                AnalyzerMatch::new("angular_route", r.path.clone(), pos(source, &r.path), file_type, fp())
                    .with_provenance(Provenance::Other)
                    .with_severity(Severity::Info)
                    .with_metadata(serde_json::json!({
                        "chunk_id": r.chunk_id,
                        "module_name": r.module_name,
                        "is_lazy": r.is_lazy,
                    })),
            );
        }
    }
    out
}

/// RxJS: operator usage inventory (the reactive plumbing under Angular HttpClient
/// / async-pipe / NgRx effects).
pub fn rxjs(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    // `.pipe(` alone is a false-positive magnet — Node streams pipe, and the
    // operator names (map/filter/tap/…) are Array/lodash methods. Require a
    // framework-exclusive RxJS signal before reporting any operators.
    let has_rxjs = source.contains("rxjs")
        || source.contains("BehaviorSubject")
        || source.contains("ReplaySubject")
        || source.contains("rxjs/operators");
    if !has_rxjs || !(source.contains(".pipe(") || source.contains("pipe(")) {
        return Vec::new();
    }
    let fp = || file_path.map(str::to_string);
    rxjs_annotator::extract_operator_usage(source)
        .into_iter()
        .map(|(op, count)| {
            // Position on the member-access form `.op` (present in both `n.op(`
            // and `(0,n.op)(`), which avoids landing on an unrelated substring
            // occurrence of the bare operator name.
            let needle = format!(".{op}");
            AnalyzerMatch::new("rxjs_operator", op.clone(), pos(source, &needle), file_type, fp())
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "count": count }))
        })
        .collect()
}

/// single-spa micro-frontend registry (the `mfe_registry_extractor` targets
/// single-spa, not webpack Module Federation — that is `fw_module_federation`).
pub fn single_spa(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("registerApplication") || source.contains("singleSpa") || source.contains("single-spa")) {
        return Vec::new();
    }
    let r = mfe_registry_extractor::analyze_mfe_registry(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for app in &r.registrations {
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
                // Registry-wide security posture (otherwise dropped on the floor).
                "has_sri": r.has_sri,
                "has_csp_nonce": r.has_csp_nonce,
                "shared_auth_scope": r.shared_auth_scope,
            })),
        );
    }
    // Dynamic `createElement("script")` injections are the security-bearing
    // signal of this extractor: an MFE that loads remote bundles at runtime with
    // no SRI/integrity is a supply-chain surface. Surface each one, escalating
    // severity when neither the injection site nor the registry carries an
    // integrity check.
    for si in &r.script_injections {
        let unverified = !si.has_security_attrs && !r.has_sri;
        out.push(
            AnalyzerMatch::new(
                "single_spa_script_injection",
                si.url_pattern.clone(),
                pos(source, &si.url_pattern),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(if unverified { Severity::Medium } else { Severity::Info })
            .with_metadata(serde_json::json!({
                "method": si.method,
                "has_security_attrs": si.has_security_attrs,
                "has_sri": r.has_sri,
                "has_csp_nonce": r.has_csp_nonce,
            })),
        );
    }
    for ep in &r.config_endpoints {
        out.push(
            AnalyzerMatch::new(
                "single_spa_config_endpoint",
                ep.clone(),
                pos(source, ep),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low),
        );
    }
    out
}

/// LaunchDarkly: feature-flag keys + client-side SDK key / environment.
pub fn launchdarkly(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
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
    let r = launchdarkly_extractor::extract_feature_flags(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for flag in &r.flags {
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
    if let Some(key) = &r.config.sdk_key {
        out.push(
            AnalyzerMatch::new("launchdarkly_client_id", key.clone(), pos(source, key), file_type, fp())
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Medium),
        );
    }
    if let Some(env) = &r.config.environment {
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
        let src = r#"ldclient.variation("new-checkout-flow", false);"#;
        let matches = launchdarkly(src, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "new-checkout-flow"),
            "expected LaunchDarkly flag from lowercase ldclient call: {matches:?}"
        );
    }

    #[test]
    fn launchdarkly_adapter_runs_on_camelcase_ldclient_variation() {
        let src = r#"ldClient.variation("new-checkout-flow", false);"#;
        let matches = launchdarkly(src, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "new-checkout-flow"),
            "expected LaunchDarkly flag from camelCase ldClient call: {matches:?}"
        );
    }

    #[test]
    fn launchdarkly_adapter_runs_on_typed_variation_helper() {
        let src = r#"client.boolVariation("typed-flag", false);"#;
        let matches = launchdarkly(src, Some("flags.js"), FileType::Js);
        assert!(
            matches
                .iter()
                .any(|m| m.match_kind == "launchdarkly_flag" && m.match_value == "typed-flag"),
            "expected LaunchDarkly flag from typed variation helper: {matches:?}"
        );
    }
}

/// HTTP interceptor chains: injected headers + token-storage locations.
pub fn http_interceptor(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("intercept")
        || source.contains("setHeaders")
        || source.contains("HttpInterceptor")
        || source.contains("Authorization"))
    {
        return Vec::new();
    }
    let r = http_interceptor_extractor::analyze_interceptor_chain(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for h in &r.headers {
        out.push(
            AnalyzerMatch::new(
                "http_header_injection",
                h.header.clone(),
                pos(source, &h.header),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "value_source": h.value_source,
                "condition": h.condition,
                "security_note": h.security_note,
            })),
        );
    }
    for t in &r.token_storage {
        out.push(
            AnalyzerMatch::new(
                "http_token_storage",
                t.token_name.clone(),
                pos(source, &t.key),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "storage_type": t.storage_type,
                "key": t.key,
            })),
        );
    }
    out
}
