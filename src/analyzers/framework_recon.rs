//! Framework / build-tool recon dispatcher.
//!
//! Aggregates the per-framework signature analyzers (`fw_*`) — and adapters over
//! the previously compile-dead extractors in [`crate::ast_deobfuscate`] — into a
//! single flat `Vec<AnalyzerMatch>`, so framework structure (routes, components,
//! store slices, endpoints, feature-flag keys, module/chunk graphs, telemetry
//! DSNs) surfaces in the same recon output as secrets/sinks/paths.
//!
//! Wired into [`super::run_all`] behind `AnalyzerConfig::framework_recon`.

use super::match_kind::{AnalyzerMatch, FileType};

mod adapters;

/// Run every framework recon analyzer over `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    // Native framework analyzers (regex → AnalyzerMatch, uniform API).
    out.extend(super::fw_react::analyze(source, file_path, file_type));
    out.extend(super::fw_vue::analyze(source, file_path, file_type));
    out.extend(super::fw_svelte::analyze(source, file_path, file_type));
    out.extend(super::fw_redux::analyze(source, file_path, file_type));
    out.extend(super::fw_zustand::analyze(source, file_path, file_type));
    out.extend(super::fw_vite::analyze(source, file_path, file_type));
    out.extend(super::fw_rollup::analyze(source, file_path, file_type));
    out.extend(super::fw_module_federation::analyze(source, file_path, file_type));
    out.extend(super::fw_sentry::analyze(source, file_path, file_type));
    out.extend(super::fw_preact::analyze(source, file_path, file_type));
    out.extend(super::fw_solid::analyze(source, file_path, file_type));
    out.extend(super::fw_next_rsc::analyze(source, file_path, file_type));

    // Adapters over the (now-compiled) ast_deobfuscate framework extractors.
    out.extend(adapters::ngrx(source, file_path, file_type));
    out.extend(adapters::angular(source, file_path, file_type));
    out.extend(adapters::rxjs(source, file_path, file_type));
    out.extend(adapters::single_spa(source, file_path, file_type));
    out.extend(adapters::launchdarkly(source, file_path, file_type));
    out.extend(adapters::http_interceptor(source, file_path, file_type));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatcher_runs_clean_on_empty() {
        assert!(analyze("", None, FileType::Js).is_empty());
    }

    #[test]
    fn dispatcher_surfaces_react_and_ngrx() {
        let src = concat!(
            r#"var n=Symbol.for("react.transitional.element");"#,
            r#"const Xs=(0,n.createAction)("[Auth] Login Success");"#,
        );
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "react_element_factory"));
        assert!(m.iter().any(|x| x.match_kind == "ngrx_action"));
    }
}
