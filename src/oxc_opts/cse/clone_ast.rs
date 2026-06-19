use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    BinaryExpression, Expression, ExpressionStatement, IdentifierReference, NumericLiteral, Statement, StringLiteral,
    VariableDeclaration, VariableDeclarator,
};
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use super::{CommonSubexpressionElimination, Ctx};

impl CommonSubexpressionElimination {
    pub(super) fn clone_statement<'a>(&self, stmt: &Statement<'a>, ctx: &mut Ctx<'a>) -> Statement<'a> {
        match stmt {
            Statement::ExpressionStatement(expr_stmt) => {
                Statement::ExpressionStatement(ctx.ast.alloc(ExpressionStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: expr_stmt.span,
                    expression: Self::clone_expression(&expr_stmt.expression, ctx),
                }))
            }
            Statement::VariableDeclaration(var_decl) => {
                let mut new_decls = ctx.ast.vec();
                for decl in &var_decl.declarations {
                    let new_id = decl.id.clone_in(ctx.ast.allocator);
                    new_decls.push(VariableDeclarator {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: decl.span,
                        kind: var_decl.kind,
                        id: new_id,
                        type_annotation: None,
                        init: decl.init.as_ref().map(|e| Self::clone_expression(e, ctx)),
                        definite: decl.definite,
                    });
                }
                Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: var_decl.span,
                    kind: var_decl.kind,
                    declarations: new_decls,
                    declare: var_decl.declare,
                }))
            }
            _ => stmt.clone_in(ctx.ast.allocator),
        }
    }

    pub(super) fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        match expr {
            Expression::Identifier(ident) => Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                node_id: Cell::new(NodeId::DUMMY),
                span: ident.span,
                name: ctx.ast.ident(ident.name.as_str()),
                reference_id: None.into(),
            })),
            Expression::NumericLiteral(lit) => Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: lit.span,
                value: lit.value,
                raw: lit.raw,
                base: lit.base,
            })),
            Expression::StringLiteral(lit) => Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: lit.span,
                value: ctx.ast.str(lit.value.as_str()),
                raw: None,
                lone_surrogates: false,
            })),
            Expression::BinaryExpression(bin) => Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: bin.span,
                left: Self::clone_expression(&bin.left, ctx),
                operator: bin.operator,
                right: Self::clone_expression(&bin.right, ctx),
            })),
            _ => expr.clone_in(ctx.ast.allocator),
        }
    }
}
