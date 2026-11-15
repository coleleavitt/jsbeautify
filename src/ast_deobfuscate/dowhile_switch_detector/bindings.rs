use oxc_ast::ast::{IdentifierReference, Program};
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::value::StateValue;
use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::with_scope::redirectable_symbols;

/// What the analysis knows about the program's bindings.
pub(super) struct Bindings<'s> {
    pub scoping: &'s Scoping,
    redirectable: FxHashSet<SymbolId>,
    source_type: SourceType,
}

impl<'s> Bindings<'s> {
    pub fn new(program: &Program<'_>, scoping: &'s Scoping) -> Self {
        Self {
            scoping,
            redirectable: redirectable_symbols(program, scoping),
            source_type: program.source_type,
        }
    }

    /// The binding `id` resolves to.
    pub fn symbol_of(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        self.scoping.get_reference(id.reference_id.get()?).symbol_id()
    }

    /// Whether `symbol` changes only through its declaration and the
    /// references `oxc_semantic` resolves to it: no redeclaration assigns
    /// it, and no `with` object or Annex B block function stands in for it.
    pub fn is_tracked(&self, symbol: SymbolId) -> bool {
        !self.redirectable.contains(&symbol) && self.scoping.symbol_redeclarations(symbol).is_empty()
    }

    pub fn is_written(&self, symbol: SymbolId) -> bool {
        self.scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .any(|&reference| self.scoping.get_reference(reference).is_write())
    }

    /// Whether another script can reach `symbol` by name: a top-level
    /// binding of a classic script.
    pub fn is_shared(&self, symbol: SymbolId) -> bool {
        is_global_binding(self.source_type, self.scoping, symbol)
    }

    /// Whether `symbol` holds what its initialiser stored for as long as
    /// the program reads it: nothing in the program writes it, and no other
    /// script can.
    pub fn is_constant(&self, symbol: SymbolId) -> bool {
        self.is_tracked(symbol)
            && !self.is_written(symbol)
            && (!self.is_shared(symbol) || self.scoping.symbol_flags(symbol).is_const_variable())
    }
}

/// Where a declarator sits: its statement list, and its statement and
/// declarator index there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Position {
    pub list: usize,
    pub statement: usize,
    pub declarator: usize,
}

impl Position {
    /// Whether the declarator at `self` runs before the one at `other`
    /// every time the statement list runs.
    fn precedes(self, other: Self) -> bool {
        let order = |position: Self| (position.statement, position.declarator);
        self.list == other.list && order(self) < order(other)
    }
}

/// Bindings declared with a constant initialiser that nothing else writes.
#[derive(Default)]
pub(super) struct Constants(FxHashMap<SymbolId, (Position, StateValue)>);

impl Constants {
    pub fn insert(&mut self, symbol: SymbolId, declared: Position, value: StateValue) {
        self.0.insert(symbol, (declared, value));
    }

    /// The value `id` reads in code that only runs after the declarator at
    /// `after` has: that of a constant declared before `after` in the same
    /// statement list, which has been initialised by then. A binding
    /// declared elsewhere may still be uninitialised, or in its temporal
    /// dead zone.
    pub fn lookup(&self, bindings: &Bindings<'_>, id: &IdentifierReference<'_>, after: Position) -> Option<StateValue> {
        let (declared, value) = self.0.get(&bindings.symbol_of(id)?)?;
        declared.precedes(after).then(|| value.clone())
    }
}
