use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Argument, Expression, IdentifierName, NumberBase, Statement, VariableDeclarator};
use oxc_span::SPAN;
use oxc_syntax::scope::ScopeFlags;

mod argument;

use super::{Ctx, LoopUnroller};

impl LoopUnroller {
    pub(super) fn clone_and_substitute_loop_body<'a>(
        &self,
        body: &Statement<'a>,
        loop_var: &str,
        value: i64,
        ctx: &mut Ctx<'a>,
    ) -> Option<Statement<'a>> {
        match body {
            Statement::BlockStatement(block) => {
                let mut new_stmts = oxc_allocator::Vec::new_in(&ctx.ast);
                for stmt in &block.body {
                    new_stmts.push(self.substitute_in_statement(stmt, loop_var, value, ctx)?);
                }
                let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
                Some(Statement::new_block_statement_with_scope_id(
                    SPAN, new_stmts, scope_id, ctx,
                ))
            }
            Statement::ExpressionStatement(_) => self.substitute_in_statement(body, loop_var, value, ctx),
            _ => None,
        }
    }

    fn substitute_in_statement<'a>(
        &self,
        stmt: &Statement<'a>,
        loop_var: &str,
        value: i64,
        ctx: &mut Ctx<'a>,
    ) -> Option<Statement<'a>> {
        match stmt {
            Statement::ExpressionStatement(expr_stmt) => {
                let expression = self.substitute_in_expression(&expr_stmt.expression, loop_var, value, ctx)?;
                Some(Statement::new_expression_statement(SPAN, expression, ctx))
            }
            Statement::VariableDeclaration(var_decl) => {
                let mut declarations = oxc_allocator::Vec::new_in(&ctx.ast);
                for decl in &var_decl.declarations {
                    let init = match &decl.init {
                        Some(init) => Some(self.substitute_in_expression(init, loop_var, value, ctx)?),
                        None => None,
                    };
                    declarations.push(VariableDeclarator::new(
                        SPAN,
                        decl.id.clone_in(ctx.ast.allocator()),
                        None,
                        init,
                        decl.definite,
                        ctx,
                    ));
                }
                Some(Statement::new_variable_declaration(
                    SPAN,
                    var_decl.kind,
                    declarations,
                    var_decl.declare,
                    ctx,
                ))
            }
            _ => None,
        }
    }

    fn substitute_in_expression<'a>(
        &self,
        expr: &Expression<'a>,
        loop_var: &str,
        value: i64,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        match expr {
            Expression::Identifier(ident) => {
                if ident.name == loop_var {
                    Some(Expression::new_numeric_literal(
                        SPAN,
                        value as f64,
                        None,
                        NumberBase::Decimal,
                        ctx,
                    ))
                } else {
                    Some(Expression::new_identifier(SPAN, ident.name.as_str(), ctx))
                }
            }
            Expression::CallExpression(call) => {
                let callee = self.substitute_in_expression(&call.callee, loop_var, value, ctx)?;
                let mut arguments = oxc_allocator::Vec::new_in(&ctx.ast);
                for arg in &call.arguments {
                    if let Argument::SpreadElement(_) = arg {
                        return None;
                    }
                    arguments.push(self.substitute_in_argument(arg, loop_var, value, ctx)?);
                }
                Some(Expression::new_call_expression(
                    SPAN,
                    callee,
                    None,
                    arguments,
                    call.optional,
                    ctx,
                ))
            }
            Expression::BinaryExpression(bin) => {
                let left = self.substitute_in_expression(&bin.left, loop_var, value, ctx)?;
                let right = self.substitute_in_expression(&bin.right, loop_var, value, ctx)?;
                Some(Expression::new_binary_expression(SPAN, left, bin.operator, right, ctx))
            }
            Expression::StaticMemberExpression(member) => {
                let object = self.substitute_in_expression(&member.object, loop_var, value, ctx)?;
                let property = IdentifierName::new(SPAN, member.property.name.as_str(), ctx);
                Some(Expression::new_static_member_expression(
                    SPAN,
                    object,
                    property,
                    member.optional,
                    ctx,
                ))
            }
            Expression::ComputedMemberExpression(member) => {
                let object = self.substitute_in_expression(&member.object, loop_var, value, ctx)?;
                let property = self.substitute_in_expression(&member.expression, loop_var, value, ctx)?;
                Some(Expression::new_computed_member_expression(
                    SPAN,
                    object,
                    property,
                    member.optional,
                    ctx,
                ))
            }
            Expression::NumericLiteral(lit) => {
                Some(Expression::new_numeric_literal(SPAN, lit.value, lit.raw, lit.base, ctx))
            }
            Expression::StringLiteral(lit) => Some(Expression::new_string_literal(SPAN, lit.value.as_str(), None, ctx)),
            _ => None,
        }
    }
}
