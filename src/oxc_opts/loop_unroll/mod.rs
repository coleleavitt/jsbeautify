//! Loop Unrolling Optimization
//!
//! Transforms constant-bounded for loops into unrolled statements.
//! Safety constraints: numeric literal bounds, increment by 1, max 10 iterations.

mod substitute;
#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BinaryOperator, BindingPattern, Expression, ForStatement, ForStatementInit, Program, Statement, UpdateOperator,
};
use oxc_semantic::SemanticBuilder;
use oxc_span::SPAN;
use oxc_syntax::scope::ScopeFlags;
use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};

use crate::oxc_opts::state::OptimizationState;

pub type Ctx<'a> = TraverseCtx<'a, OptimizationState>;

pub struct LoopUnroller {
    changed: bool,
}

impl LoopUnroller {
    #[must_use]
    pub const fn new() -> Self {
        Self { changed: false }
    }

    pub fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;

        let state = OptimizationState::new();
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, allocator);

        traverse_mut_with_ctx(self, program, &mut ctx);

        self.changed
    }

    fn try_unroll_for_loop<'a>(
        &mut self,
        stmt: &ForStatement<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<oxc_allocator::Vec<'a, Statement<'a>>> {
        let init = stmt.init.as_ref()?;
        let ForStatementInit::VariableDeclaration(var_decl) = init else {
            return None;
        };

        if var_decl.declarations.len() != 1 {
            return None;
        }

        let decl = &var_decl.declarations[0];

        let loop_var_name = match &decl.id {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str().to_string(),
            _ => return None,
        };

        let init_value = match &decl.init {
            Some(Expression::NumericLiteral(lit)) => lit.value as i64,
            _ => return None,
        };

        let test = stmt.test.as_ref()?;
        let Expression::BinaryExpression(bin) = test else {
            return None;
        };

        let limit = match &bin.right {
            Expression::NumericLiteral(lit) => lit.value as i64,
            _ => return None,
        };

        let increment = match &stmt.update {
            Some(Expression::UpdateExpression(update)) => {
                if update.operator == UpdateOperator::Increment {
                    1
                } else if update.operator == UpdateOperator::Decrement {
                    -1
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        let iterations = match bin.operator {
            BinaryOperator::LessThan => limit.checked_sub(init_value)?,
            BinaryOperator::LessEqualThan => limit.checked_sub(init_value)?.checked_add(1)?,
            _ => return None,
        };

        if iterations <= 0 || iterations > 10 {
            return None;
        }

        let mut unrolled_stmts = ctx.ast.vec();

        for i in 0..iterations {
            let current_value = init_value.checked_add(i.checked_mul(increment)?)?;

            let body_clone = self.clone_and_substitute_loop_body(&stmt.body, &loop_var_name, current_value, ctx)?;

            unrolled_stmts.push(body_clone);
        }

        self.changed = true;
        Some(unrolled_stmts)
    }
}

impl Default for LoopUnroller {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, OptimizationState> for LoopUnroller {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::ForStatement(for_stmt) = stmt
            && let Some(unrolled) = self.try_unroll_for_loop(for_stmt, ctx)
        {
            let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
            let block = ctx.ast.block_statement_with_scope_id(SPAN, unrolled, scope_id);
            *stmt = Statement::BlockStatement(ctx.ast.alloc(block));
        }
    }
}
