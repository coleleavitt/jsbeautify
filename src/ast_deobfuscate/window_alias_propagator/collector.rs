use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, ForInStatement, ForOfStatement, SimpleAssignmentTarget, UpdateExpression,
    VariableDeclarator,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::utils::is_global_like;

pub struct WindowAliasCollector {
    candidates: FxHashSet<SymbolId>,
    poisoned: FxHashSet<SymbolId>,
}

impl WindowAliasCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: FxHashSet::default(),
            poisoned: FxHashSet::default(),
        }
    }

    #[must_use]
    pub fn aliases(self) -> FxHashSet<SymbolId> {
        &self.candidates - &self.poisoned
    }
}

impl Default for WindowAliasCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasCollector {
    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
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
        match decl.init.as_ref() {
            Some(init) if is_global_like(init) => {
                self.candidates.insert(symbol_id);
            }
            _ => {
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
        if assign.operator.is_assign() && is_global_like(&assign.right) {
            self.candidates.insert(symbol_id);
        } else {
            self.poisoned.insert(symbol_id);
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
