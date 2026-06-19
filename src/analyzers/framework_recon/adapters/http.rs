use crate::analyzers::match_kind::{AnalyzerMatch, FileType, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::http_interceptor_extractor;

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    if !(source.contains("intercept")
        || source.contains("setHeaders")
        || source.contains("HttpInterceptor")
        || source.contains("Authorization"))
    {
        return Vec::new();
    }
    let result = http_interceptor_extractor::analyze_interceptor_chain(source);
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);
    for header in &result.headers {
        out.push(
            AnalyzerMatch::new(
                "http_header_injection",
                header.header.clone(),
                pos(source, &header.header),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "value_source": header.value_source,
                "condition": header.condition,
                "security_note": header.security_note,
            })),
        );
    }
    for token in &result.token_storage {
        out.push(
            AnalyzerMatch::new(
                "http_token_storage",
                token.token_name.clone(),
                pos(source, &token.key),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "storage_type": token.storage_type,
                "key": token.key,
            })),
        );
    }
    out
}
