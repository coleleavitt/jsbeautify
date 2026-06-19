use oxc_ast::ast::Expression;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod arithmetic;
mod constructors;
mod logical;

pub struct AlgebraicSimplifier {
    changed: bool,
}

flag_pass!(AlgebraicSimplifier, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for AlgebraicSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::BinaryExpression(binary) => self
                .try_simplify_self_subtract(binary, ctx)
                .or_else(|| self.try_simplify_multiply_zero(binary, ctx))
                .or_else(|| self.try_simplify_self_divide(binary, ctx))
                .or_else(|| self.try_simplify_self_modulo(binary, ctx))
                .or_else(|| self.try_simplify_self_xor(binary, ctx))
                .or_else(|| self.try_simplify_add_zero(binary, ctx))
                .or_else(|| self.try_simplify_multiply_one(binary, ctx))
                .or_else(|| self.try_simplify_divide_one(binary, ctx)),
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
