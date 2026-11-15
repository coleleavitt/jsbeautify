use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

pub struct DeobfuscateState {
    pub changed: bool,
    pub dispatchers: FxHashMap<String, DispatcherInfo>,
}

impl DeobfuscateState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            changed: false,
            dispatchers: FxHashMap::default(),
        }
    }
}

impl Default for DeobfuscateState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct DispatcherInfo {
    pub var_name: String,
    pub symbol: Option<SymbolId>,
    pub functions: FxHashMap<String, FunctionInfo>,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub key: String,
    pub return_value: Option<ReturnValue>,
}

#[derive(Debug, Clone)]
pub enum ReturnValue {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
}
