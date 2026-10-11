//! Bindings that are only ever called.

use oxc_ast::ast::{Argument, CallExpression, Expression, Program};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;

/// Never-reassigned, file-local bindings whose every reference is the callee of a plain
/// call `f(a, …)` without spread arguments, with the argument counts used.
///
/// A pass may replace such a binding's calls and delete its declaration
/// without leaving a dangling reference.
#[must_use]
pub fn plain_call_arities(program: &Program<'_>, scoping: &Scoping) -> FxHashMap<SymbolId, FxHashSet<usize>> {
    let mut calls = PlainCalls {
        scoping,
        calls: FxHashMap::default(),
    };
    calls.visit_program(program);
    calls
        .calls
        .into_iter()
        .filter_map(|(symbol, (count, arities))| {
            let arities = arities?;
            (count == scoping.get_resolved_reference_ids(symbol).len()
                && !scoping.symbol_is_mutated(symbol)
                && scoping.symbol_redeclarations(symbol).is_empty()
                && !is_global_binding(program.source_type, scoping, symbol))
            .then_some((symbol, arities))
        })
        .collect()
}

struct PlainCalls<'s> {
    scoping: &'s Scoping,
    /// Per callee: the number of calls and their argument counts, `None` once
    /// a call spreads its arguments.
    calls: FxHashMap<SymbolId, (usize, Option<FxHashSet<usize>>)>,
}

impl<'a> Visit<'a> for PlainCalls<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &it.callee
            && let Some(symbol) = self.scoping.get_reference(callee.reference_id()).symbol_id()
        {
            let (count, arities) = self
                .calls
                .entry(symbol)
                .or_insert_with(|| (0, Some(FxHashSet::default())));
            *count += 1;
            if it.arguments.iter().any(|a| matches!(a, Argument::SpreadElement(_))) {
                *arities = None;
            } else if let Some(arities) = arities {
                arities.insert(it.arguments.len());
            }
        }
        walk::walk_call_expression(self, it);
    }
}
