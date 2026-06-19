//! Dead-case pruner for do-while-switch dispatchers.
//!
//! Consumes a [`DoWhileDispatcherMap`] (from [`super::dowhile_switch_detector`])
//! and removes switch cases that are unreachable from any entry point.
//!
//! Reachability is computed by:
//! 1. Scanning call sites to find entry states (e.g. `Ql(Q4, [...])`)
//! 2. Following state transitions from each reachable case to its successors
//! 3. Iterating until fixpoint
//! 4. Removing cases whose labels are not in the reachable set

use oxc_ast::ast::{BindingPattern, Expression, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use super::dowhile_switch_detector::{DoWhileDispatcherMap, StateTransition};
use super::state::DeobfuscateState;

mod entries;
mod prune;
mod reachability;
mod switch_body;

use entries::collect_entry_states;
use reachability::compute_reachability;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DoWhileSwitchCleaner {
    dispatchers: DoWhileDispatcherMap,
    reachable_per_dispatcher: FxHashMap<String, FxHashSet<String>>,
    pruned_cases: usize,
}

impl DoWhileSwitchCleaner {
    #[must_use]
    pub fn new(dispatchers: DoWhileDispatcherMap, program: &Program<'_>) -> Self {
        let entry_states = collect_entry_states(program, &dispatchers);
        let reachable_per_dispatcher = compute_reachability(&dispatchers, &entry_states);
        Self {
            dispatchers,
            reachable_per_dispatcher,
            pruned_cases: 0,
        }
    }

    #[must_use]
    pub const fn pruned_cases(&self) -> usize {
        self.pruned_cases
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DoWhileSwitchCleaner {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                let Some(id) = &func.id else { return };
                let name = id.name.as_str();
                self.try_prune_dispatcher(name, func, ctx);
            }
            Statement::VariableDeclaration(decl) => {
                for d in &mut decl.declarations {
                    let BindingPattern::BindingIdentifier(var_id) = &d.id else {
                        continue;
                    };
                    let var_name = var_id.name.as_str().to_string();
                    let Some(Expression::FunctionExpression(func)) = &mut d.init else {
                        continue;
                    };
                    self.try_prune_dispatcher_mut(&var_name, func.as_mut(), ctx);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
