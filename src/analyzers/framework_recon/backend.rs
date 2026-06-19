use super::{AnalyzerMatch, FileType};

type AnalyzeFn = fn(&str, Option<&str>, FileType) -> Vec<AnalyzerMatch>;

pub(super) trait FrameworkBackend {
    fn analyze(&self, source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch>;
}

#[derive(Clone, Copy)]
pub(super) struct FunctionBackend {
    analyze: AnalyzeFn,
}

impl FunctionBackend {
    pub(super) const fn new(analyze: AnalyzeFn) -> Self {
        Self { analyze }
    }
}

impl FrameworkBackend for FunctionBackend {
    fn analyze(&self, source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
        (self.analyze)(source, file_path, file_type)
    }
}

pub(super) fn run_all(
    backends: &[FunctionBackend],
    source: &str,
    file_path: Option<&str>,
    file_type: FileType,
    out: &mut Vec<AnalyzerMatch>,
) {
    for backend in backends {
        out.extend(backend.analyze(source, file_path, file_type));
    }
}
