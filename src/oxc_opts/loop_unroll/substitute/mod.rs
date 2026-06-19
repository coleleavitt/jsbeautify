use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    Argument, BinaryExpression, CallExpression, ComputedMemberExpression, Expression, ExpressionStatement,
    IdentifierName, IdentifierReference, NumberBase, NumericLiteral, Statement, StaticMemberExpression, StringLiteral,
    VariableDeclaration, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::scope::ScopeFlags;
use std::cell::Cell;

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
                let mut new_stmts = ctx.ast.vec();

                for stmt in &block.body {
                    {
                        let new_stmt = self.substitute_in_statement(stmt, loop_var, value, ctx)?;
                        new_stmts.push(new_stmt);
                    }
                }

                let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
                let block = ctx.ast.block_statement_with_scope_id(SPAN, new_stmts, scope_id);
                Some(Statement::BlockStatement(ctx.ast.alloc(block)))
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
                let new_expr = self.substitute_in_expression(&expr_stmt.expression, loop_var, value, ctx)?;
                Some(Statement::ExpressionStatement(ctx.ast.alloc(ExpressionStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    expression: new_expr,
                })))
            }
            Statement::VariableDeclaration(var_decl) => {
                let mut new_decls = ctx.ast.vec();
                for decl in &var_decl.declarations {
                    let new_init = if let Some(init) = &decl.init {
                        Some(self.substitute_in_expression(init, loop_var, value, ctx)?)
                    } else {
                        None
                    };

                    let new_id = decl.id.clone_in(ctx.ast.allocator);

                    new_decls.push(VariableDeclarator {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        kind: var_decl.kind,
                        id: new_id,
                        type_annotation: None,
                        init: new_init,
                        definite: decl.definite,
                    });
                }

                Some(Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    kind: var_decl.kind,
                    declarations: new_decls,
                    declare: var_decl.declare,
                })))
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
                if ident.name.as_str() == loop_var {
                    Some(Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        value: value as f64,
                        raw: None,
                        base: NumberBase::Decimal,
                    })))
                } else {
                    Some(Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        name: ctx.ast.ident(ident.name.as_str()),
                        reference_id: None.into(),
                    })))
                }
            }
            Expression::CallExpression(call) => {
                let new_callee = self.substitute_in_expression(&call.callee, loop_var, value, ctx)?;
                let mut new_args = ctx.ast.vec();

                for arg in &call.arguments {
                    if let Argument::SpreadElement(_) = arg {
                        return None;
                    }
                    let new_arg = self.substitute_in_argument(arg, loop_var, value, ctx)?;
                    new_args.push(new_arg);
                }

                Some(Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee: new_callee,
                    type_arguments: None,
                    arguments: new_args,
                    optional: call.optional,
                    pure: false,
                })))
            }
            Expression::BinaryExpression(bin) => {
                let left = self.substitute_in_expression(&bin.left, loop_var, value, ctx)?;
                let right = self.substitute_in_expression(&bin.right, loop_var, value, ctx)?;

                Some(Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    left,
                    operator: bin.operator,
                    right,
                })))
            }
            Expression::StaticMemberExpression(static_mem) => {
                let object = self.substitute_in_expression(&static_mem.object, loop_var, value, ctx)?;

                Some(Expression::StaticMemberExpression(ctx.ast.alloc(
                    StaticMemberExpression {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        object,
                        property: IdentifierName {
                            node_id: Cell::new(NodeId::DUMMY),
                            span: SPAN,
                            name: ctx.ast.ident(static_mem.property.name.as_str()),
                        },
                        optional: static_mem.optional,
                    },
                )))
            }
            Expression::ComputedMemberExpression(computed) => {
                let object = self.substitute_in_expression(&computed.object, loop_var, value, ctx)?;
                let property = self.substitute_in_expression(&computed.expression, loop_var, value, ctx)?;

                Some(Expression::ComputedMemberExpression(ctx.ast.alloc(
                    ComputedMemberExpression {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        object,
                        expression: property,
                        optional: computed.optional,
                    },
                )))
            }
            Expression::NumericLiteral(lit) => Some(Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: lit.value,
                raw: lit.raw,
                base: lit.base,
            }))),
            Expression::StringLiteral(lit) => Some(Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: ctx.ast.str(lit.value.as_str()),
                raw: None,
                lone_surrogates: false,
            }))),
            _ => None,
        }
    }
}
