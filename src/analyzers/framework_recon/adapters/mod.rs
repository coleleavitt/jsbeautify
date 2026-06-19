mod angular;
mod common;
mod http;
mod launchdarkly;
mod ngrx;
mod rxjs;
mod single_spa;

use crate::analyzers::match_kind::{AnalyzerMatch, FileType};

use super::backend::{FunctionBackend, run_all};

const BACKENDS: &[FunctionBackend] = &[
    FunctionBackend::new(ngrx::analyze),
    FunctionBackend::new(angular::analyze),
    FunctionBackend::new(rxjs::analyze),
    FunctionBackend::new(single_spa::analyze),
    FunctionBackend::new(launchdarkly::analyze),
    FunctionBackend::new(http::analyze),
];

pub(super) fn analyze(source: &str, file_path: Option<&str>, file_type: FileType, out: &mut Vec<AnalyzerMatch>) {
    run_all(BACKENDS, source, file_path, file_type, out);
}
