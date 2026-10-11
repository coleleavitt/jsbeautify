use oxc_syntax::symbol::SymbolId;

#[derive(Debug, Clone)]
pub struct LookupForwarderInfo {
    pub accessor_name: String,
    pub accessor_symbol: Option<SymbolId>,
}
