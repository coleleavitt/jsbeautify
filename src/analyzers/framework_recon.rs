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
mod backend;

use backend::{FunctionBackend, run_all};

const NATIVE_BACKENDS: &[FunctionBackend] = &[
    FunctionBackend::new(super::fw_react::analyze),
    FunctionBackend::new(super::fw_vue::analyze),
    FunctionBackend::new(super::fw_svelte::analyze),
    FunctionBackend::new(super::fw_redux::analyze),
    FunctionBackend::new(super::fw_zustand::analyze),
    FunctionBackend::new(super::fw_vite::analyze),
    FunctionBackend::new(super::fw_rollup::analyze),
    FunctionBackend::new(super::fw_module_federation::analyze),
    FunctionBackend::new(super::fw_sentry::analyze),
    FunctionBackend::new(super::fw_preact::analyze),
    FunctionBackend::new(super::fw_solid::analyze),
    FunctionBackend::new(super::fw_next_rsc::analyze),
    FunctionBackend::new(super::fw_webpack::analyze),
];

/// Run every framework recon analyzer over `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    run_all(NATIVE_BACKENDS, source, file_path, file_type, &mut out);
    adapters::analyze(source, file_path, file_type, &mut out);
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
