use oxc_ast::ast::{AssignmentExpression, AssignmentTarget, BindingPattern, Program, VariableDeclarator};
use oxc_span::SourceType;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::{redirectable_symbols, scopes_in_with_bodies};

use super::Ctx;
use super::utils::{GlobalObject, global_object};

/// Finds aliases of a global object: bindings whose every declaration is
/// `var X = window` (or `self`, `global`, `globalThis`) and whose every write
/// is a plain `X = window`. Any other write (`X = other`, `X++`, `[X] = …`,
/// `for (X of …)`, a parameter set through `arguments`, a function
/// declaration of the same name) means `X` may hold something else.
#[derive(Default)]
pub struct WindowAliasCollector {
    candidates: FxHashMap<SymbolId, GlobalObject>,
    /// Per candidate, its declarations of the form `var X = window`.
    alias_declarations: FxHashMap<SymbolId, usize>,
    /// The references of `X = window` assignments.
    alias_writes: FxHashSet<ReferenceId>,
    with_scopes: FxHashSet<ScopeId>,
    poisoned: FxHashSet<SymbolId>,
    source_type: SourceType,
}

impl WindowAliasCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Each alias with the global object it holds.
    #[must_use]
    pub fn aliases(mut self) -> FxHashMap<SymbolId, GlobalObject> {
        self.candidates.retain(|symbol, _| !self.poisoned.contains(symbol));
        self.candidates
    }

    fn add_candidate(&mut self, symbol: SymbolId, global: GlobalObject) {
        self.candidates
            .entry(symbol)
            .and_modify(|held| *held = held.merge(global))
            .or_insert(global);
    }

    /// `expr`'s global object, unless a `with` object may stand in for it.
    fn global_object(&self, expr: &oxc_ast::ast::Expression<'_>, ctx: &Ctx<'_>) -> Option<GlobalObject> {
        let oxc_ast::ast::Expression::Identifier(id) = expr else {
            return None;
        };
        let scope = ctx.scoping().get_reference(id.reference_id.get()?).scope_id();
        if self.with_scopes.contains(&scope) {
            return None;
        }
        global_object(expr, ctx.scoping())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.with_scopes = scopes_in_with_bodies(ctx.scoping());
        self.source_type = program.source_type;
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let Some(symbol_id) = id.symbol_id.get() else {
            return;
        };
        if let Some(global) = decl.init.as_ref().and_then(|init| self.global_object(init, ctx)) {
            self.add_candidate(symbol_id, global);
            *self.alias_declarations.entry(symbol_id).or_default() += 1;
        }
    }

    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left else {
            return;
        };
        let Some(reference_id) = id.reference_id.get() else {
            return;
        };
        let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id() else {
            return;
        };
        // Only a *simple* `X = window` promotes X. A compound assignment
        // (`X += window` computes `X + window`, a number) is another write.
        if assign.operator.is_assign()
            && let Some(global) = self.global_object(&assign.right, ctx)
        {
            self.add_candidate(symbol_id, global);
            self.alias_writes.insert(reference_id);
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let scoping = ctx.scoping();
        // `with` objects, Annex B functions and direct `eval` may stand in for
        // an alias, and other scripts may assign a classic script's global.
        let redirectable = redirectable_symbols(program, scoping);
        for &symbol in self.candidates.keys() {
            let declarations = 1 + scoping.symbol_redeclarations(symbol).len();
            let only_alias_writes = scoping.get_resolved_reference_ids(symbol).iter().all(|&reference| {
                !scoping.get_reference(reference).is_write() || self.alias_writes.contains(&reference)
            });
            if self.alias_declarations.get(&symbol).copied().unwrap_or_default() != declarations
                || !only_alias_writes
                || redirectable.contains(&symbol)
                || is_global_binding(self.source_type, scoping, symbol)
            {
                self.poisoned.insert(symbol);
            }
        }
    }
}
