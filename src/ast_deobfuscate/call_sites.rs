//! Bindings that are only ever called.

use oxc_ast::ast::{Argument, CallExpression, Expression, Program};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::with_scope::redirectable_symbols;

/// Never-reassigned, file-local bindings whose every reference is the callee of a plain
/// call `f(a, …)` without spread arguments, with the argument counts used.
///
/// A pass may replace such a binding's calls and delete its declaration
/// without leaving a dangling reference. Bindings an Annex B block function, a
/// `with` object or a direct `eval` may stand in for at runtime
/// ([`redirectable_symbols`]) are left out, since their calls need not reach
/// their declaration.
#[must_use]
pub fn plain_call_arities(program: &Program<'_>, scoping: &Scoping) -> FxHashMap<SymbolId, FxHashSet<usize>> {
    let mut calls = PlainCalls {
        scoping,
        calls: FxHashMap::default(),
    };
    calls.visit_program(program);
    let shadowed = redirectable_symbols(program, scoping);
    calls
        .calls
        .into_iter()
        .filter_map(|(symbol, (count, arities))| {
            let arities = arities?;
            (count == scoping.get_resolved_reference_ids(symbol).len()
                && !scoping.symbol_is_mutated(symbol)
                && scoping.symbol_redeclarations(symbol).is_empty()
                && !is_global_binding(program.source_type, scoping, symbol)
                && !shadowed.contains(&symbol))
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

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::plain_call_arities;
    use crate::ast_deobfuscate::semantic;

    /// The names of the bindings `plain_call_arities` returns, sorted.
    fn called(code: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let mut names: Vec<_> = plain_call_arities(&program, &scoping)
            .into_keys()
            .map(|symbol| scoping.symbol_name(symbol).to_string())
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn leaves_out_bindings_a_direct_eval_can_see() {
        for code in [
            "function f(a) { return a; } eval('f = 0'); f(1);",
            "function f(a) { return a; } function g() { eval('var f'); return f(1); } g();",
            "function f(a) { return a; } function g() { (eval)('f(2)'); } f(1); g();",
        ] {
            assert!(called(code).is_empty(), "{code}: {:?}", called(code));
        }
    }

    #[test]
    fn keeps_bindings_outside_the_direct_eval_scope_chain() {
        let code = "function g() { eval('1'); } (function () { function f(a) { return a; } f(1); })(); g();";
        // `g` shares the program scope with the eval; `f` is in a sibling scope.
        assert_eq!(called(code), ["f"]);
    }
}
