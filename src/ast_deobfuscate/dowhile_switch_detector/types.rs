use rustc_hash::FxHashMap;

pub type DoWhileDispatcherMap = FxHashMap<String, DoWhileDispatcherInfo>;

#[derive(Debug)]
pub struct DoWhileDispatcherInfo {
    pub name: String,
    pub alt_name: Option<String>,
    pub state_param: String,
    pub args_param: String,
    pub exit_sentinel: String,
    pub cases: Vec<DoWhileCaseInfo>,
    pub has_default: bool,
}

#[derive(Debug)]
pub struct DoWhileCaseInfo {
    pub label: String,
    pub body_statement_count: usize,
    pub transition: StateTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    AddAssign,
    SubAssign,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StateTransition {
    Sequential(String),
    Conditional(String, String),
    Compound { op: CompoundOp, rhs_name: String },
    Return,
    LoopExit,
    Unknown,
}
