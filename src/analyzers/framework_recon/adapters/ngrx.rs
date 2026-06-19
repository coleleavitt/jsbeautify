use crate::analyzers::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::ngrx_extractor;

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("createAction") || source.contains("createEffect") || source.contains(".dispatch(")) {
        return Vec::new();
    }
    let result = ngrx_extractor::extract_ngrx_patterns(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    for action in &result.actions {
        out.push(
            AnalyzerMatch::new(
                "ngrx_action",
                action.action_type.clone(),
                pos(source, &action.action_type),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "minified_name": action.minified_name,
                "has_props": action.has_props,
            })),
        );
    }
    for effect in &result.effects {
        out.push(
            AnalyzerMatch::new(
                "ngrx_effect",
                effect.trigger_actions.join(", "),
                PositionRange::new(1, 0, 1, 0),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "triggers": effect.trigger_actions,
                "dispatches": effect.dispatches,
            })),
        );
    }
    for dispatch in &result.store_dispatches {
        out.push(
            AnalyzerMatch::new(
                "ngrx_dispatch",
                dispatch.clone(),
                pos(source, dispatch),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }
    out
}
