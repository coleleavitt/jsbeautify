use oxc_ast::ast::{Argument, CallExpression, Expression, IdentifierReference};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::bindings::{Bindings, Constants, Position};
use super::value::{StateValue, evaluate};

/// The calls of each candidate dispatcher: `F(STATE, …)` and
/// `F.call(THIS, STATE, …)`, through the binding it is assigned to or its
/// own name.
pub(super) struct CallSites<'b, 's> {
    bindings: &'b Bindings<'s>,
    constants: &'b Constants,
    /// Candidate index by the bindings that name it.
    targets: FxHashMap<SymbolId, usize>,
    declared: Vec<Position>,
    /// Per candidate, the state values of its calls.
    pub entries: Vec<Vec<StateValue>>,
    /// Per candidate, whether a call passes a state that is not constant.
    pub unknown_entry: Vec<bool>,
    /// The references that are the callee of a recognised call.
    pub calls: FxHashSet<ReferenceId>,
}

impl<'b, 's> CallSites<'b, 's> {
    pub fn new(
        bindings: &'b Bindings<'s>,
        constants: &'b Constants,
        targets: FxHashMap<SymbolId, usize>,
        declared: Vec<Position>,
    ) -> Self {
        let count = declared.len();
        Self {
            bindings,
            constants,
            targets,
            declared,
            entries: vec![Vec::new(); count],
            unknown_entry: vec![false; count],
            calls: FxHashSet::default(),
        }
    }

    fn record(&mut self, call: &CallExpression<'_>) {
        let (callee, state_index) = match &call.callee {
            Expression::Identifier(callee) => (callee, 0),
            Expression::StaticMemberExpression(member) if member.property.name == "call" => {
                let Expression::Identifier(callee) = &member.object else {
                    return;
                };
                (callee, 1)
            }
            _ => return,
        };
        let Some(&target) = self
            .bindings
            .symbol_of(callee)
            .and_then(|symbol| self.targets.get(&symbol))
        else {
            return;
        };
        let Some(reference) = callee.reference_id.get() else {
            return;
        };
        self.calls.insert(reference);
        let declared = self.declared[target];
        let lookup = |id: &IdentifierReference<'_>| self.constants.lookup(self.bindings, id, declared);
        let state = call
            .arguments
            .iter()
            .take(state_index + 1)
            .map(Argument::as_expression)
            .collect::<Option<Vec<_>>>()
            .and_then(|arguments| arguments.get(state_index).copied())
            .and_then(|state| evaluate(state, &lookup));
        match state {
            Some(value) => self.entries[target].push(value),
            None => self.unknown_entry[target] = true,
        }
    }
}

impl<'a> Visit<'a> for CallSites<'_, '_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.record(it);
        walk::walk_call_expression(self, it);
    }
}
