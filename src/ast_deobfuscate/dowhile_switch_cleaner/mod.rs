//! Dead-case pruner for do-while-switch dispatchers.
//!
//! Takes the dispatchers [`super::dowhile_switch_detector`] finds and
//! removes the switch cases no call can reach:
//! 1. the state values the calls pass are reachable;
//! 2. a reachable value selects the first case whose label is that value,
//!    and that case is reachable;
//! 3. a reachable case makes the values its body can assign reachable, and
//!    the next case too when its body can fall through;
//! 4. repeat until nothing new is reachable.
//!
//! A dispatcher with no known call keeps every case, and an unreachable
//! case that declares a binding stays: the reachable cases share its scope.

use oxc_ast::ast::{BindingPattern, Expression, Program, VariableDeclarator};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use super::dowhile_switch_detector::detect;
use super::state::DeobfuscateState;

mod prune;
mod reachability;

use prune::Prune;
use reachability::reachable_cases;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DoWhileSwitchCleaner {
    dispatchers: usize,
    prunes: FxHashMap<SymbolId, Prune>,
    pruned_cases: usize,
}

impl DoWhileSwitchCleaner {
    /// Plans the pruning of `program`, which `scoping` describes.
    #[must_use]
    pub fn new(program: &Program<'_>, scoping: &Scoping) -> Self {
        let dispatchers = detect(program, scoping);
        let count = dispatchers.len();
        let prunes = dispatchers
            .into_iter()
            .filter_map(|(symbol, dispatcher)| {
                let reached = reachable_cases(&dispatcher);
                reached.contains(&false).then_some((
                    symbol,
                    Prune {
                        loop_index: dispatcher.loop_index,
                        switch_index: dispatcher.switch_index,
                        reached,
                    },
                ))
            })
            .collect();
        Self {
            dispatchers: count,
            prunes,
            pruned_cases: 0,
        }
    }

    /// How many dispatchers the analysis understood.
    #[must_use]
    pub const fn dispatchers(&self) -> usize {
        self.dispatchers
    }

    /// Whether some dispatcher has a case to prune.
    #[must_use]
    pub fn has_dead_cases(&self) -> bool {
        !self.prunes.is_empty()
    }

    #[must_use]
    pub const fn pruned_cases(&self) -> usize {
        self.pruned_cases
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DoWhileSwitchCleaner {
    fn exit_variable_declarator(&mut self, declarator: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let Some(prune) = id.symbol_id.get().and_then(|symbol| self.prunes.remove(&symbol)) else {
            return;
        };
        if let Some(Expression::FunctionExpression(func)) = &mut declarator.init {
            self.pruned_cases += prune.apply(func);
        }
    }
}

#[cfg(test)]
mod tests;
