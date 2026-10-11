//! Collision-free name assignment for the webpack rename plan.
//!
//! Renaming symbol `S`, declared in scope `X`, to `N` is accepted only when:
//!
//! 1. no other binding named `N` is declared in `X` (no redeclaration);
//! 2. no reference inside `X`'s subtree resolves to an outer binding or global
//!    named `N` (it would be captured by `S`);
//! 3. no binding named `N` sits between a reference to `S` and `X` (it would
//!    capture that reference).
//!
//! Otherwise `N$1`, `N$2`, … are tried. Names are tracked as they are
//! assigned, so later plan entries see earlier renames.

use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Every symbol visible from one of `scopes` (declared in it or an ancestor).
pub(super) fn symbols_visible_from(scopes: &[ScopeId], scoping: &Scoping) -> FxHashSet<SymbolId> {
    if scopes.is_empty() {
        return FxHashSet::default();
    }
    let visible: FxHashSet<ScopeId> = scopes.iter().flat_map(|&s| scoping.scope_ancestors(s)).collect();
    scoping
        .symbol_ids()
        .filter(|&sym| visible.contains(&scoping.symbol_scope_id(sym)))
        .collect()
}

struct Planner<'s> {
    scoping: &'s Scoping,
    /// Base names of the plan; only these (and their `$N` forms) are tracked.
    wanted: FxHashSet<String>,
    /// `(scope, name)` for every binding declared in `scope`.
    bound: FxHashSet<(ScopeId, String)>,
    /// `(scope, name)` when a reference to `name` inside `scope` resolves to a
    /// binding outside it, or to a global.
    free: FxHashSet<(ScopeId, String)>,
}

impl<'s> Planner<'s> {
    fn new(scoping: &'s Scoping, wanted: FxHashSet<String>) -> Self {
        let mut planner = Self {
            scoping,
            wanted,
            bound: FxHashSet::default(),
            free: FxHashSet::default(),
        };
        for symbol in scoping.symbol_ids() {
            let name = scoping.symbol_name(symbol);
            if planner.is_tracked(name) {
                planner.declare(symbol, name);
            }
        }
        for (name, references) in scoping.root_unresolved_references() {
            if !planner.is_tracked(name) {
                continue;
            }
            for &reference in references {
                let scope = scoping.get_reference(reference).scope_id();
                for s in scoping.scope_ancestors(scope) {
                    planner.free.insert((s, name.to_string()));
                }
            }
        }
        planner
    }

    fn is_tracked(&self, name: &str) -> bool {
        self.wanted.contains(name)
            || name.rsplit_once('$').is_some_and(|(base, n)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && self.wanted.contains(base)
            })
    }

    fn reference_scopes(&self, symbol: SymbolId) -> impl Iterator<Item = ScopeId> + '_ {
        self.scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .map(|&reference| self.scoping.get_reference(reference).scope_id())
    }

    /// Record `symbol` as currently named `name`.
    fn declare(&mut self, symbol: SymbolId, name: &str) {
        let home = self.scoping.symbol_scope_id(symbol);
        self.bound.insert((home, name.to_string()));
        let scopes: Vec<ScopeId> = self.reference_scopes(symbol).collect();
        for scope in scopes {
            for s in self.scoping.scope_ancestors(scope).take_while(|&s| s != home) {
                self.free.insert((s, name.to_string()));
            }
        }
    }

    fn is_safe(&self, symbol: SymbolId, name: &str) -> bool {
        let home = self.scoping.symbol_scope_id(symbol);
        let key = (home, name.to_string());
        if self.bound.contains(&key) || self.free.contains(&key) {
            return false;
        }
        self.reference_scopes(symbol).all(|scope| {
            self.scoping
                .scope_ancestors(scope)
                .take_while(|&s| s != home)
                .all(|s| !self.bound.contains(&(s, name.to_string())))
        })
    }

    fn assign(&mut self, symbol: SymbolId, desired: &str) -> String {
        let mut name = desired.to_string();
        let mut n = 1u32;
        while !self.is_safe(symbol, &name) {
            name = format!("{desired}${n}");
            n += 1;
        }
        self.declare(symbol, &name);
        name
    }
}

/// Resolve `plan` (in priority order) into final, collision-free names,
/// skipping `guarded` symbols and symbols already carrying the desired name.
pub(super) fn assign_names(
    plan: Vec<(SymbolId, String)>,
    guarded: &FxHashSet<SymbolId>,
    scoping: &Scoping,
) -> FxHashMap<SymbolId, String> {
    let plan: Vec<_> = plan
        .into_iter()
        .filter(|(symbol, name)| !guarded.contains(symbol) && scoping.symbol_name(*symbol) != name)
        .collect();
    let mut planner = Planner::new(scoping, plan.iter().map(|(_, name)| name.clone()).collect());
    plan.into_iter()
        .map(|(symbol, desired)| (symbol, planner.assign(symbol, &desired)))
        .collect()
}
