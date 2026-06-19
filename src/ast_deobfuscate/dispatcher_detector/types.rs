use rustc_hash::FxHashMap;

/// Maps dispatcher function name → info about its cases.
pub type DispatcherMap = FxHashMap<String, DispatcherInfo>;

/// Metadata about a single CFF dispatcher function.
#[derive(Debug)]
pub struct DispatcherInfo {
    pub name: String,
    pub alt_name: Option<String>,
    pub state_param: String,
    pub args_param: String,
    pub cases: FxHashMap<String, CaseInfo>,
    pub has_default: bool,
}

/// Metadata about a single case clause inside a dispatcher.
#[derive(Debug)]
pub struct CaseInfo {
    pub state_name: String,
    pub body_statement_count: usize,
}
