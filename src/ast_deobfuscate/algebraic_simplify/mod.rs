use oxc_ast::ast::Expression;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod logical;

pub struct AlgebraicSimplifier {
    rewrites: usize,
}

counter_pass!(AlgebraicSimplifier, rewrites, rewrites);

impl<'a> Traverse<'a, DeobfuscateState> for AlgebraicSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::LogicalExpression(logical) => self
                .try_simplify_logical_and(logical, ctx)
                .or_else(|| self.try_simplify_logical_or(logical, ctx)),
            _ => None,
        };

        if let Some(new_expr) = replacement {
            *expr = new_expr;
        }
    }
}

#[cfg(test)]
mod tests;
