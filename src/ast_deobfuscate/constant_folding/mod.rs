use oxc_ast::ast::{BinaryOperator, Expression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod fold;
mod values;

pub struct ConstantFolder {
    changed: bool,
}

flag_pass!(ConstantFolder, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for ConstantFolder {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::BinaryExpression(binary) => {
                if matches!(
                    binary.operator,
                    BinaryOperator::StrictEquality
                        | BinaryOperator::Equality
                        | BinaryOperator::StrictInequality
                        | BinaryOperator::Inequality
                        | BinaryOperator::LessThan
                        | BinaryOperator::LessEqualThan
                        | BinaryOperator::GreaterThan
                        | BinaryOperator::GreaterEqualThan
                ) {
                    self.try_fold_comparison(binary, ctx)
                } else {
                    self.try_fold_binary(binary, ctx)
                }
            }
            Expression::LogicalExpression(logical) => self.try_fold_logical(logical, ctx),
            Expression::UnaryExpression(unary) => self.try_fold_unary(unary, ctx),
            _ => None,
        };

        if let Some(new_expr) = replacement {
            *expr = new_expr;
        }
    }
}

#[cfg(test)]
mod tests;
