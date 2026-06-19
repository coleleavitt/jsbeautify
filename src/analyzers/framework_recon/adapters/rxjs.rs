use crate::analyzers::match_kind::{AnalyzerMatch, FileType, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::rxjs_annotator;

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
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
            let needle = format!(".{op}");
            AnalyzerMatch::new("rxjs_operator", op, pos(source, &needle), file_type, fp())
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({ "count": count }))
        })
        .collect()
}
