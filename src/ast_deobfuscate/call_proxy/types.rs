use oxc_syntax::symbol::SymbolId;

#[derive(Debug, Clone)]
pub struct CallProxyInfo {
    pub target_name: String,
    pub target_symbol: Option<SymbolId>,
    pub params: usize,
}
