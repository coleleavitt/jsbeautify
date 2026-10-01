//! Static framework and build-tool reconnaissance.
//!
//! The dispatcher only parses and pattern-matches the supplied text. It never
//! evaluates JavaScript, loads a bundle, or applies a source transformation.

use serde::{Deserialize, Serialize};

use crate::ast_deobfuscate::{
    angular_metadata, angular_route_extractor, http_interceptor_extractor, launchdarkly_extractor,
    mfe_registry_extractor, ngrx_extractor, rxjs_annotator, transmit_sdk_decoder, webpack_module_resolver,
    webpack_runtime_decoder,
};
use crate::recon_budget::ReconBudget;

/// Hostile-input limits for framework reconnaissance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisLimits {
    pub max_source_bytes: usize,
    pub max_scan_bytes_per_extractor: usize,
    pub max_findings_per_section: usize,
    pub max_report_bytes: usize,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 32 * 1024 * 1024,
            max_scan_bytes_per_extractor: 8 * 1024 * 1024,
            max_findings_per_section: 10_000,
            max_report_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedFinding {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AngularReport {
    pub version: Option<String>,
    pub components: Vec<NamedFinding>,
    pub directives: Vec<NamedFinding>,
    pub pipes: Vec<NamedFinding>,
    pub injectables: Vec<NamedFinding>,
    pub modules: Vec<NamedFinding>,
    pub routes: Vec<RouteFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFinding {
    pub path: String,
    pub chunk_id: Option<u32>,
    pub module_name: Option<String>,
    pub lazy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HttpReport {
    pub headers: Vec<NamedFinding>,
    pub token_storage: Vec<NamedFinding>,
    pub crypto_auth: bool,
    pub risk_headers: bool,
    pub policy_bypass_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeatureFlagReport {
    pub flags: Vec<NamedFinding>,
    pub sdk_key: Option<String>,
    pub environment: Option<String>,
    pub total_evaluations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MicroFrontendReport {
    pub registrations: Vec<NamedFinding>,
    pub script_injections: Vec<NamedFinding>,
    pub config_endpoints: Vec<String>,
    pub sri: bool,
    pub csp_nonce: bool,
    pub shared_auth_scope: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NgrxReport {
    pub actions: Vec<NamedFinding>,
    pub effects: Vec<NamedFinding>,
    pub store_dispatches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransmitReport {
    pub sdk_version: Option<String>,
    pub api_calls: Vec<NamedFinding>,
    pub fingerprint_fields: Vec<NamedFinding>,
    pub webauthn: bool,
    pub cross_device: bool,
    pub debug_token: bool,
    pub risk_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebpackModuleFinding {
    pub id: u32,
    pub package: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebpackReport {
    pub require_variable: Option<String>,
    pub modules: Vec<WebpackModuleFinding>,
    pub total_modules: usize,
    pub unresolved_modules: usize,
}

/// Stable aggregate output from every framework reconnaissance extractor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkReconReport {
    pub schema_version: u32,
    pub angular: AngularReport,
    pub http: HttpReport,
    pub feature_flags: FeatureFlagReport,
    pub micro_frontends: MicroFrontendReport,
    pub ngrx: NgrxReport,
    pub rxjs_operators: Vec<NamedFinding>,
    pub transmit: TransmitReport,
    pub webpack: WebpackReport,
    pub truncated: bool,
    pub diagnostics: Vec<String>,
}

impl Default for FrameworkReconReport {
    fn default() -> Self {
        Self {
            schema_version: 1,
            angular: AngularReport::default(),
            http: HttpReport::default(),
            feature_flags: FeatureFlagReport::default(),
            micro_frontends: MicroFrontendReport::default(),
            ngrx: NgrxReport::default(),
            rxjs_operators: Vec::new(),
            transmit: TransmitReport::default(),
            webpack: WebpackReport::default(),
            truncated: false,
            diagnostics: Vec::new(),
        }
    }
}

/// Analyze source with the default hostile-input limits.
#[must_use]
pub fn analyze(source: &str) -> FrameworkReconReport {
    analyze_with_limits(source, AnalysisLimits::default())
}

/// Analyze source without evaluating or transforming it.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn analyze_with_limits(source: &str, limits: AnalysisLimits) -> FrameworkReconReport {
    if source.len() > limits.max_source_bytes {
        return FrameworkReconReport {
            truncated: true,
            diagnostics: vec![format!(
                "framework reconnaissance skipped: source is {} bytes; limit is {} bytes",
                source.len(),
                limits.max_source_bytes
            )],
            ..FrameworkReconReport::default()
        };
    }

    let scan_bytes = limits.max_scan_bytes_per_extractor.min(source.len());
    let mut scan_end = scan_bytes;
    while !source.is_char_boundary(scan_end) {
        scan_end -= 1;
    }
    let bounded_source = &source[..scan_end];

    let mut recon_budget = ReconBudget::new(limits.max_report_bytes);
    let angular = angular_metadata::extract_angular_metadata_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let routes = angular_route_extractor::extract_routes_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let http = http_interceptor_extractor::analyze_interceptor_chain_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let flags = launchdarkly_extractor::extract_feature_flags_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let mfe = mfe_registry_extractor::analyze_mfe_registry_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let ngrx = ngrx_extractor::extract_ngrx_patterns_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let transmit = transmit_sdk_decoder::analyze_transmit_sdk_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );
    let webpack = webpack_module_resolver::resolve_module_ids_with_budget(
        bounded_source,
        limits.max_findings_per_section,
        &mut recon_budget,
    );

    let angular_report = build_angular_report(angular, routes);

    let mut report = FrameworkReconReport {
        angular: angular_report,
        http: HttpReport {
            headers: http
                .headers
                .into_iter()
                .map(|header| NamedFinding {
                    name: header.header,
                    detail: Some(header.value_source),
                })
                .collect(),
            token_storage: http
                .token_storage
                .into_iter()
                .map(|token| NamedFinding {
                    name: token.token_name,
                    detail: Some(format!("{}:{}", token.storage_type, token.key)),
                })
                .collect(),
            crypto_auth: http.has_crypto_auth,
            risk_headers: http.has_risk_headers,
            policy_bypass_flags: http.policy_bypass_flags,
        },
        feature_flags: FeatureFlagReport {
            flags: flags
                .flags
                .into_iter()
                .map(|flag| NamedFinding {
                    name: flag.key,
                    detail: flag.default_value,
                })
                .collect(),
            sdk_key: flags.config.sdk_key,
            environment: flags.config.environment,
            total_evaluations: flags.total_evaluations,
        },
        micro_frontends: MicroFrontendReport {
            registrations: mfe
                .registrations
                .into_iter()
                .map(|registration| NamedFinding {
                    name: registration.name,
                    detail: registration.route_pattern.or(registration.source_url),
                })
                .collect(),
            script_injections: mfe
                .script_injections
                .into_iter()
                .map(|injection| NamedFinding {
                    name: injection.method,
                    detail: Some(injection.url_pattern),
                })
                .collect(),
            config_endpoints: mfe.config_endpoints,
            sri: mfe.has_sri,
            csp_nonce: mfe.has_csp_nonce,
            shared_auth_scope: mfe.shared_auth_scope,
        },
        ngrx: NgrxReport {
            actions: ngrx
                .actions
                .into_iter()
                .map(|action| NamedFinding {
                    name: action.action_type,
                    detail: action.minified_name,
                })
                .collect(),
            effects: ngrx
                .effects
                .into_iter()
                .map(|effect| NamedFinding {
                    name: effect.trigger_actions.join(", "),
                    detail: effect.dispatches.then_some("dispatches".to_owned()),
                })
                .collect(),
            store_dispatches: ngrx.store_dispatches,
        },
        rxjs_operators: rxjs_annotator::extract_operator_usage_with_budget(
            bounded_source,
            limits.max_findings_per_section,
            &mut recon_budget,
        )
        .into_iter()
        .map(|(name, count)| NamedFinding {
            name,
            detail: Some(count.to_string()),
        })
        .collect(),
        transmit: TransmitReport {
            sdk_version: transmit.sdk_version,
            api_calls: transmit
                .api_calls
                .into_iter()
                .map(|call| NamedFinding {
                    name: call.endpoint,
                    detail: Some(call.purpose),
                })
                .collect(),
            fingerprint_fields: transmit
                .fingerprint_fields
                .into_iter()
                .map(|field| NamedFinding {
                    name: field.field_name,
                    detail: Some(field.category.to_owned()),
                })
                .collect(),
            webauthn: transmit.capabilities.has_webauthn(),
            cross_device: transmit.capabilities.has_cross_device(),
            debug_token: transmit.capabilities.has_debug_token(),
            risk_session: transmit.capabilities.risk_session_detected(),
        },
        webpack: WebpackReport {
            require_variable: webpack_runtime_decoder::detect_require_var_with_budget(
                bounded_source,
                &mut recon_budget,
            ),
            modules: webpack
                .resolved
                .into_values()
                .map(|module| WebpackModuleFinding {
                    id: module.id,
                    package: module.package,
                    confidence: format!("{:?}", module.confidence).to_lowercase(),
                })
                .collect(),
            total_modules: webpack.total_modules,
            unresolved_modules: webpack.unresolved_count,
        },
        ..FrameworkReconReport::default()
    };

    if recon_budget.exhausted() {
        report.truncated = true;
        report
            .diagnostics
            .push("framework reconnaissance payload budget exhausted".to_owned());
    }
    if bounded_source.len() < source.len() {
        report.truncated = true;
        report.diagnostics.push(format!(
            "framework reconnaissance scanned {} of {} source bytes",
            bounded_source.len(),
            source.len()
        ));
    }
    constrain_payload_allocations(&mut report, limits.max_report_bytes);
    sort_report(&mut report);
    enforce_limits(&mut report, limits);
    report
}

fn build_angular_report(
    angular: angular_metadata::AngularExtractionResult,
    routes: angular_route_extractor::RouteExtractionResult,
) -> AngularReport {
    let mut report = AngularReport {
        version: angular.version.map(|version| version.to_string()),
        routes: routes
            .routes
            .into_iter()
            .map(|route| RouteFinding {
                path: route.path,
                chunk_id: route.chunk_id,
                module_name: route.module_name,
                lazy: route.is_lazy,
            })
            .collect(),
        ..AngularReport::default()
    };
    for definition in angular.definitions {
        let finding = NamedFinding {
            name: definition.class_name,
            detail: Some(definition.selector),
        };
        match definition.kind {
            angular_metadata::AngularDefKind::Component => report.components.push(finding),
            angular_metadata::AngularDefKind::Directive => report.directives.push(finding),
            angular_metadata::AngularDefKind::Pipe => report.pipes.push(finding),
            angular_metadata::AngularDefKind::Injectable => report.injectables.push(finding),
            angular_metadata::AngularDefKind::NgModule => report.modules.push(finding),
        }
    }
    report
}

fn constrain_payload_allocations(report: &mut FrameworkReconReport, max_bytes: usize) {
    fn constrain(value: &mut String, remaining: &mut usize) {
        if value.len() > *remaining {
            let mut end = *remaining;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            value.truncate(end);
        }
        *remaining = remaining.saturating_sub(value.len());
        value.shrink_to_fit();
    }

    fn constrain_optional(value: &mut Option<String>, remaining: &mut usize) {
        if let Some(value) = value {
            constrain(value, remaining);
        }
    }

    fn constrain_named(findings: &mut [NamedFinding], remaining: &mut usize) {
        for finding in findings {
            constrain(&mut finding.name, remaining);
            constrain_optional(&mut finding.detail, remaining);
        }
    }

    let mut remaining = max_bytes;
    constrain_optional(&mut report.angular.version, &mut remaining);
    for findings in [
        &mut report.angular.components,
        &mut report.angular.directives,
        &mut report.angular.pipes,
        &mut report.angular.injectables,
        &mut report.angular.modules,
        &mut report.http.headers,
        &mut report.http.token_storage,
        &mut report.feature_flags.flags,
        &mut report.micro_frontends.registrations,
        &mut report.micro_frontends.script_injections,
        &mut report.ngrx.actions,
        &mut report.ngrx.effects,
        &mut report.rxjs_operators,
        &mut report.transmit.api_calls,
        &mut report.transmit.fingerprint_fields,
    ] {
        constrain_named(findings, &mut remaining);
    }
    for route in &mut report.angular.routes {
        constrain(&mut route.path, &mut remaining);
        constrain_optional(&mut route.module_name, &mut remaining);
    }
    for value in [
        &mut report.feature_flags.sdk_key,
        &mut report.feature_flags.environment,
        &mut report.transmit.sdk_version,
        &mut report.webpack.require_variable,
    ] {
        constrain_optional(value, &mut remaining);
    }
    for values in [
        &mut report.http.policy_bypass_flags,
        &mut report.micro_frontends.config_endpoints,
        &mut report.ngrx.store_dispatches,
    ] {
        for value in values {
            constrain(value, &mut remaining);
        }
    }
    for module in &mut report.webpack.modules {
        constrain(&mut module.package, &mut remaining);
        constrain(&mut module.confidence, &mut remaining);
    }
}

#[cfg(test)]
fn retained_payload_bytes(report: &FrameworkReconReport) -> usize {
    fn strings(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::String(value) => value.len(),
            serde_json::Value::Array(values) => values.iter().map(strings).sum(),
            serde_json::Value::Object(values) => values.values().map(strings).sum(),
            _ => 0,
        }
    }
    let mut report = report.clone();
    report.diagnostics.clear();
    let json = serde_json::to_value(report).expect("report serializes");
    strings(&json)
}

fn sort_report(report: &mut FrameworkReconReport) {
    for findings in [
        &mut report.angular.components,
        &mut report.angular.directives,
        &mut report.angular.pipes,
        &mut report.angular.injectables,
        &mut report.angular.modules,
        &mut report.http.headers,
        &mut report.http.token_storage,
        &mut report.feature_flags.flags,
        &mut report.micro_frontends.registrations,
        &mut report.micro_frontends.script_injections,
        &mut report.ngrx.actions,
        &mut report.ngrx.effects,
        &mut report.rxjs_operators,
        &mut report.transmit.api_calls,
        &mut report.transmit.fingerprint_fields,
    ] {
        findings.sort_by(|left, right| left.name.cmp(&right.name).then(left.detail.cmp(&right.detail)));
    }
    report.angular.routes.sort_by(|left, right| left.path.cmp(&right.path));
    report.webpack.modules.sort_by_key(|module| module.id);
    report.http.policy_bypass_flags.sort();
    report.micro_frontends.config_endpoints.sort();
    report.ngrx.store_dispatches.sort();
}

fn enforce_limits(report: &mut FrameworkReconReport, limits: AnalysisLimits) {
    let mut truncated = false;
    macro_rules! cap {
        ($items:expr) => {
            if $items.len() >= limits.max_findings_per_section {
                $items.truncate(limits.max_findings_per_section);
                truncated = true;
            }
        };
    }
    cap!(report.angular.components);
    cap!(report.angular.directives);
    cap!(report.angular.pipes);
    cap!(report.angular.injectables);
    cap!(report.angular.modules);
    cap!(report.angular.routes);
    cap!(report.http.headers);
    cap!(report.http.token_storage);
    cap!(report.http.policy_bypass_flags);
    cap!(report.feature_flags.flags);
    cap!(report.micro_frontends.registrations);
    cap!(report.micro_frontends.script_injections);
    cap!(report.micro_frontends.config_endpoints);
    cap!(report.ngrx.actions);
    cap!(report.ngrx.effects);
    cap!(report.ngrx.store_dispatches);
    cap!(report.rxjs_operators);
    cap!(report.transmit.api_calls);
    cap!(report.transmit.fingerprint_fields);
    cap!(report.webpack.modules);

    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() > limits.max_report_bytes) {
        *report = FrameworkReconReport {
            truncated: true,
            diagnostics: vec!["framework reconnaissance report exceeded payload limit; findings omitted".to_owned()],
            ..FrameworkReconReport::default()
        };
        return;
    }
    if truncated {
        report.truncated = true;
        report
            .diagnostics
            .push("framework reconnaissance findings truncated by section limit".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_multiple_extractors_into_stable_json() {
        let source = r#"
            const C = ɵɵdefineComponent({selectors:[["app-root"]]});
            const action = createAction("[Auth] Login");
            client.variation("checkout-enabled", false);
            stream.pipe((0, rx.map)(x => x), (0, rx.filter)(Boolean));
        "#;
        let report = analyze(source);
        let json = serde_json::to_value(&report).expect("serialize report");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["feature_flags"]["flags"][0]["name"], "checkout-enabled");
        assert_eq!(json["ngrx"]["actions"][0]["name"], "[Auth] Login");
        assert!(json["rxjs_operators"].as_array().is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn oversized_input_is_not_dispatched() {
        let limits = AnalysisLimits {
            max_source_bytes: 4,
            ..AnalysisLimits::default()
        };
        let report = analyze_with_limits("angular.module('would-match')", limits);
        assert!(report.truncated);
        assert!(report.angular.version.is_none());
        assert!(report.diagnostics[0].contains("skipped"));
    }

    #[test]
    fn webpack_modules_are_serialized_in_numeric_order() {
        let source = "({20:function(e){e.exports='react'},3:function(e){e.exports='lodash'}})";
        let report = analyze(source);
        assert!(report.webpack.modules.windows(2).all(|pair| pair[0].id <= pair[1].id));
    }
    #[test]
    fn hostile_repeated_findings_are_bounded_during_extraction() {
        use std::fmt::Write as _;
        let source = (0..200).fold(String::new(), |mut source, index| {
            write!(source, r#"client.variation("flag-{index}", false);"#).expect("write string");
            source
        });
        let limits = AnalysisLimits {
            max_findings_per_section: 3,
            max_report_bytes: usize::MAX,
            ..AnalysisLimits::default()
        };

        let extracted =
            launchdarkly_extractor::extract_feature_flags_with_limit(&source, limits.max_findings_per_section);
        assert_eq!(extracted.flags.len(), limits.max_findings_per_section);

        let report = analyze_with_limits(&source, limits);
        assert_eq!(report.feature_flags.flags.len(), limits.max_findings_per_section);
        assert!(report.truncated);
    }

    #[test]
    fn zero_finding_limit_allocates_no_intermediate_findings() {
        let source = r#"client.variation("one", false); client.variation("two", true);"#;
        let extracted = launchdarkly_extractor::extract_feature_flags_with_budget(source, 0, &mut ReconBudget::new(0));
        assert!(extracted.flags.is_empty());

        let report = analyze_with_limits(
            source,
            AnalysisLimits {
                max_findings_per_section: 0,
                max_report_bytes: usize::MAX,
                ..AnalysisLimits::default()
            },
        );
        assert_eq!(report.feature_flags.flags, []);
        assert!(report.truncated);
    }
    #[test]
    fn large_repeated_payload_is_bounded_before_serialization() {
        let large_key = "🦀".repeat(4_096);
        let source = format!(r#"client.variation("{large_key}", false);"#);
        let max_report_bytes = 64;
        let extracted = launchdarkly_extractor::extract_feature_flags_with_budget(
            &source,
            4,
            &mut ReconBudget::new(max_report_bytes),
        );
        let mut report = FrameworkReconReport {
            feature_flags: FeatureFlagReport {
                flags: extracted
                    .flags
                    .into_iter()
                    .map(|flag| NamedFinding {
                        name: flag.key,
                        detail: flag.default_value,
                    })
                    .collect(),
                ..FeatureFlagReport::default()
            },
            ..FrameworkReconReport::default()
        };
        constrain_payload_allocations(&mut report, max_report_bytes);

        assert!(retained_payload_bytes(&report) <= max_report_bytes);
        assert_eq!(report.feature_flags.flags, []);
    }
}
