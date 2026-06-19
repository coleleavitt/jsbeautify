use crate::analyzers::match_kind::{AnalyzerMatch, FileType, Provenance};
use crate::analyzers::regex_bank::Severity;
use crate::ast_deobfuscate::{angular_metadata, angular_route_extractor};

use super::common::pos;

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    if source.contains("defineComponent")
        || source.contains("ɵcmp")
        || source.contains("ɵɵdefineComponent")
        || source.contains("definePipe")
        || source.contains("defineInjectable")
    {
        let meta = angular_metadata::extract_angular_metadata(source);
        for component in &meta.definitions {
            out.push(
                AnalyzerMatch::new(
                    "angular_component",
                    component.selector.clone(),
                    pos(source, &component.selector),
                    file_type,
                    fp(),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({
                    "class_name": component.class_name,
                    "kind": format!("{:?}", component.kind),
                    "inputs": component.inputs,
                    "outputs": component.outputs,
                    "minified_export_name": component.minified_export_name,
                    "minified_class_name": component.minified_class_name,
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
        for route in &routes.routes {
            out.push(
                AnalyzerMatch::new(
                    "angular_route",
                    route.path.clone(),
                    pos(source, &route.path),
                    file_type,
                    fp(),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info)
                .with_metadata(serde_json::json!({
                    "chunk_id": route.chunk_id,
                    "module_name": route.module_name,
                    "is_lazy": route.is_lazy,
                })),
            );
        }
    }
    out
}
