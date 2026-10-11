use oxc_syntax::symbol::SymbolId;

#[derive(Debug, Clone)]
pub struct TrampolineInfo {
    pub dispatcher: String,
    pub dispatcher_symbol: Option<SymbolId>,
    pub state: String,
    pub state_symbol: Option<SymbolId>,
    /// The trampoline is strict code, so a plain call passes `this = undefined`.
    pub strict: bool,
}
