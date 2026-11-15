use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

use super::value::StateValue;

/// Dispatchers by the binding their function expression is assigned to.
pub type DoWhileDispatcherMap = FxHashMap<SymbolId, DoWhileDispatcher>;

/// `var F = function (state, args) { … do { switch (state) { … } } while (…); }`
/// whose state only ever holds the values recorded here.
#[derive(Debug)]
pub struct DoWhileDispatcher {
    /// Index of the `do … while` among the function body's statements.
    pub loop_index: usize,
    /// Index of the `switch` among the loop body's statements, or `None`
    /// when the loop body is the `switch` itself.
    pub switch_index: Option<usize>,
    /// The `switch` cases, in source order.
    pub cases: Vec<DoWhileCase>,
    /// The state values the dispatcher is called with.
    pub entries: Vec<StateValue>,
}

#[derive(Debug)]
pub struct DoWhileCase {
    pub label: StateValue,
    /// Every value the case body can leave in the state, besides the one it
    /// was entered with.
    pub next: Vec<StateValue>,
    /// Whether running the case body can reach the next case's body.
    pub falls_through: bool,
}
