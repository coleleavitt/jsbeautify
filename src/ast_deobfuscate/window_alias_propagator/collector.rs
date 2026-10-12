use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, ForInStatement, ForOfStatement, SimpleAssignmentTarget, UpdateExpression,
    VariableDeclarator,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::utils::{GlobalObject, global_object};

pub struct WindowAliasCollector {
    candidates: FxHashMap<SymbolId, GlobalObject>,
    poisoned: FxHashSet<SymbolId>,
}

impl WindowAliasCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: FxHashMap::default(),
            poisoned: FxHashSet::default(),
        }
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
}

impl Default for WindowAliasCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasCollector {
    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let Some(symbol_id) = id.symbol_id.get() else {
            return;
        };
        // `var X = window` makes X a candidate. A `var X = other()` (or a bare
        // `var X`) poisons it: under `var` hoisting both declarations share ONE
        // SymbolId, so a later non-global redeclaration must cancel the alias.
        // A different binding named X in another scope is a distinct SymbolId.
        match decl.init.as_ref().and_then(|init| global_object(init, ctx.scoping())) {
            Some(global) => self.add_candidate(symbol_id, global),
            None => {
                self.poisoned.insert(symbol_id);
            }
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
        // (`X += window` computes `X + window`, a number) or any non-global RHS
        // means X no longer holds a pure global object and must be poisoned.
        match global_object(&assign.right, ctx.scoping()) {
            Some(global) if assign.operator.is_assign() => self.add_candidate(symbol_id, global),
            _ => {
                self.poisoned.insert(symbol_id);
            }
        }
    }

    fn enter_update_expression(&mut self, update: &mut UpdateExpression<'a>, ctx: &mut Ctx<'a>) {
        // `X++` / `--X` rebinds X to a number — it can never stay a global alias.
        let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &update.argument else {
            return;
        };
        if let Some(reference_id) = id.reference_id.get()
            && let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id()
        {
            self.poisoned.insert(symbol_id);
        }
    }

    fn enter_for_in_statement(&mut self, stmt: &mut ForInStatement<'a>, ctx: &mut Ctx<'a>) {
        poison_for_loop_target(&stmt.left, ctx, &mut self.poisoned);
    }

    fn enter_for_of_statement(&mut self, stmt: &mut ForOfStatement<'a>, ctx: &mut Ctx<'a>) {
        poison_for_loop_target(&stmt.left, ctx, &mut self.poisoned);
    }
}

/// `for (X in obj)` / `for (X of it)` rebinds the existing variable `X` on every
/// iteration, so an aliased `X` no longer holds a global object.
fn poison_for_loop_target(
    left: &oxc_ast::ast::ForStatementLeft<'_>,
    ctx: &Ctx<'_>,
    poisoned: &mut FxHashSet<SymbolId>,
) {
    let oxc_ast::ast::ForStatementLeft::AssignmentTargetIdentifier(id) = left else {
        return;
    };
    if let Some(reference_id) = id.reference_id.get()
        && let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id()
    {
        poisoned.insert(symbol_id);
    }
}
