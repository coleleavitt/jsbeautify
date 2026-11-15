//! String-array-factory dispatch inliner.
//!
//! Resolves `x19()[CONST]` and `Y49()[CONST]` patterns into literal strings.
//!
//! Detects function declarations that return a literal string array:
//!
//! ```text
//! function x19() { return ["BR","t5","B5",...]; }
//! ```
//!
//! or the self-init accessor pattern:
//!
//! ```text
//! function x19() {
//!     var vB4 = ["BR","t5","B5",...];
//!     x19 = function() { return vB4; };
//!     return vB4;
//! }
//! ```
//!
//! Then replaces `x19()[5]` with `"BE"` (the string at index 5).
//! Also resolves index constants: `var CONST = 1 + OTHER;` or `CONST = 2;`.
//!
//! Factories, constants and call sites are matched by `SymbolId` (an
//! undeclared global by its unresolved references), never by name. A
//! binding qualifies only when nothing but its own definition can give it a
//! value: see [`DispatchInlinerCollector`].

use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;
use oxc_traverse::TraverseCtx;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod collector;
mod eval;
mod rewriter;

pub use collector::DispatchInlinerCollector;
pub use rewriter::DispatchInlinerRewriter;

/// A binding an index constant is stored in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Binding {
    Symbol(SymbolId),
    /// An undeclared global, which every unresolved reference of this name reads.
    Global(String),
}

/// Where a definition's value becomes visible.
#[derive(Clone, Copy)]
struct Reach {
    /// The end of the definition statement.
    ready: u32,
    /// The scope whose body holds the definition.
    scope: ScopeId,
    /// The definition is in the program body after nothing but function
    /// declarations and declarations that run no code, so no code runs
    /// before it.
    early: bool,
}

impl Reach {
    /// Whether a use at offset `at` in `scope` runs after the definition.
    /// Textual order decides, except that a function declaration between
    /// the definition's scope and the use may be called (hoisted) before the
    /// definition runs, unless no code runs before it.
    fn reaches(&self, at: u32, scope: ScopeId, hoisting: &Hoisting) -> bool {
        at >= self.ready && (self.early || !hoisting.declaration_between(scope, self.scope))
    }
}

/// Per scope, its depth and its nearest enclosing (or own) function
/// declaration scope.
#[derive(Default)]
struct Hoisting(FxHashMap<ScopeId, (usize, Option<ScopeId>)>);

impl Hoisting {
    fn new(scoping: &Scoping, declarations: &FxHashSet<ScopeId>) -> Self {
        let mut nesting = FxHashMap::default();
        for scope in scoping.scope_descendants_from_root() {
            let mut chain = Vec::new();
            let mut current = Some(scope);
            while let Some(id) = current
                && !nesting.contains_key(&id)
            {
                chain.push(id);
                current = scoping.scope_parent_id(id);
            }
            let (mut depth, mut declaration) = current.and_then(|id| nesting.get(&id).copied()).unwrap_or((0, None));
            for id in chain.into_iter().rev() {
                depth += 1;
                if declarations.contains(&id) {
                    declaration = Some(id);
                }
                nesting.insert(id, (depth, declaration));
            }
        }
        Self(nesting)
    }

    /// Whether a function declaration encloses `inner` below its ancestor `outer`.
    fn declaration_between(&self, inner: ScopeId, outer: ScopeId) -> bool {
        let depth = |scope| self.0.get(&scope).map(|&(depth, _)| depth);
        self.0
            .get(&inner)
            .and_then(|&(_, declaration)| declaration)
            .and_then(depth)
            .is_some_and(|declaration_depth| depth(outer).is_none_or(|outer_depth| declaration_depth > outer_depth))
    }
}

struct Constant {
    value: usize,
    reach: Reach,
}

/// The index constants the collector proved.
#[derive(Default)]
pub struct IndexConstants {
    constants: FxHashMap<Binding, Constant>,
    hoisting: Hoisting,
}

impl IndexConstants {
    #[must_use]
    pub fn len(&self) -> usize {
        self.constants.len()
    }

    /// The value a read of `binding` at offset `at` in `scope` sees.
    fn get(&self, binding: &Binding, at: u32, scope: ScopeId) -> Option<usize> {
        self.constants
            .get(binding)
            .filter(|constant| constant.reach.reaches(at, scope, &self.hoisting))
            .map(|constant| constant.value)
    }
}

#[cfg(test)]
mod tests;
