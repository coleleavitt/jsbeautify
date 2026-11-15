use oxc_ast::ast::{Expression, Program, Statement, VariableDeclarator};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use rustc_hash::FxHashMap;

use super::bindings::Bindings;
use super::calls::CallSites;
use super::types::DoWhileDispatcherMap;
use super::walker::Walker;

/// The do-while dispatchers of `program` whose state values are all known,
/// keyed by the binding each function expression is assigned to.
///
/// A dispatcher qualifies only when nothing but its recognised calls can
/// run it and nothing but its recognised transitions can change its state:
/// the binding it is assigned to, and the function's own name, are only
/// ever called, with a constant state; and the state is written only by
/// `state = C`, `state += C` and `state -= C` in its case bodies. A direct
/// `eval` anywhere can do either, so it disables the analysis.
#[must_use]
pub fn detect(program: &Program<'_>, scoping: &Scoping) -> DoWhileDispatcherMap {
    let mut dispatchers = DoWhileDispatcherMap::default();
    if scoping.root_scope_flags().contains_direct_eval() || !has_candidate(program) {
        return dispatchers;
    }
    let bindings = Bindings::new(program, scoping);
    let mut walker = Walker::new(&bindings);
    walker.visit_program(program);
    if walker.candidates.is_empty() {
        return dispatchers;
    }

    let mut targets = FxHashMap::default();
    for (i, candidate) in walker.candidates.iter().enumerate() {
        targets.insert(candidate.symbol, i);
        if let Some(name) = candidate.name {
            targets.insert(name, i);
        }
    }
    let declared = walker.candidates.iter().map(|candidate| candidate.declared).collect();
    let mut calls = CallSites::new(&bindings, &walker.constants, targets, declared);
    calls.visit_program(program);

    let only_called = |symbol| {
        scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .all(|reference| calls.calls.contains(reference))
    };
    for (i, mut candidate) in walker.candidates.into_iter().enumerate() {
        let name_ok = candidate
            .name
            .is_none_or(|name| bindings.is_tracked(name) && only_called(name));
        if calls.unknown_entry[i] || !name_ok || !only_called(candidate.symbol) {
            continue;
        }
        candidate.dispatcher.entries = std::mem::take(&mut calls.entries[i]);
        dispatchers.insert(candidate.symbol, candidate.dispatcher);
    }
    dispatchers
}

/// Whether some declarator assigns a function expression whose body has a
/// `do … while` statement, the cheap precondition of a dispatcher.
fn has_candidate(program: &Program<'_>) -> bool {
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
            if let Some(Expression::FunctionExpression(func)) = &it.init
                && func.body.as_ref().is_some_and(|body| {
                    body.statements
                        .iter()
                        .any(|statement| matches!(statement, Statement::DoWhileStatement(_)))
                })
            {
                self.0 = true;
            }
            if !self.0 {
                walk::walk_variable_declarator(self, it);
            }
        }
    }
    let mut finder = Finder(false);
    finder.visit_program(program);
    finder.0
}
