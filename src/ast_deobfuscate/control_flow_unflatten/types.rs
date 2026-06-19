use rustc_hash::FxHashMap;

pub(super) enum UnflattenAction {
    Keep,
    Skip,
    UnflattenWhile(usize),
    UnflattenFor(usize),
}

#[derive(Debug, Clone)]
pub struct ControlFlowInfo {
    pub sequence_var: String,
    pub index_var: String,
    pub sequence: Vec<String>,
    pub case_statements: FxHashMap<String, Vec<usize>>,
}
