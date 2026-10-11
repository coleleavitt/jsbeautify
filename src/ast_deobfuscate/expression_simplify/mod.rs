use oxc_ast::ast::{Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod constructors;
mod rules;

pub struct ExpressionSimplifier {
    changed: bool,
}
flag_pass!(ExpressionSimplifier, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for ExpressionSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::UnaryExpression(unary) => self
                .try_simplify_double_not(unary, ctx)
                .or_else(|| self.try_simplify_plus_array(unary, ctx))
                .or_else(|| self.try_simplify_not_number(unary, ctx)),
            Expression::ComputedMemberExpression(member) => self.try_simplify_bracket_to_dot(member, ctx),
            Expression::BinaryExpression(binary) => self.try_simplify_string_concat(binary, ctx),
            _ => None,
        };

        if let Some(new_expr) = replacement {
            *expr = new_expr;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::DebuggerStatement(_) = stmt {
            eprintln!("[AST] Removing debugger statement");
            self.changed = true;
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
    }
}

#[cfg(test)]
mod tests;
