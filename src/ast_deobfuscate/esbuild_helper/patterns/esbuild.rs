use oxc_ast::ast::{ArrowFunctionExpression, Expression, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

fn nested_arrow_expression<'a, 'b>(arrow: &'a ArrowFunctionExpression<'b>) -> Option<&'a Expression<'b>> {
    let Expression::ArrowFunctionExpression(inner) = arrow.body.as_expression()? else {
        return None;
    };
    if !inner.params.items.is_empty() {
        return None;
    }
    inner.body.as_expression()
}

fn unwrap_parenthesized<'a, 'b>(expr: &'a Expression<'b>) -> &'a Expression<'b> {
    match expr {
        Expression::ParenthesizedExpression(paren) => &paren.expression,
        other => other,
    }
}

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_commonjs_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 {
            return false;
        }
        let Some(expr) = nested_arrow_expression(arrow) else {
            return false;
        };
        let Expression::SequenceExpression(seq) = unwrap_parenthesized(expr) else {
            return false;
        };
        seq.expressions.len() == 2
            && matches!(&seq.expressions[1], Expression::StaticMemberExpression(member) if member.property.name.as_str() == "exports")
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_esm_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 {
            return false;
        }
        let Some(expr) = nested_arrow_expression(arrow) else {
            return false;
        };
        let Expression::SequenceExpression(seq) = unwrap_parenthesized(expr) else {
            return false;
        };
        if seq.expressions.len() != 2 {
            return false;
        }
        let Expression::LogicalExpression(logical) = &seq.expressions[0] else {
            return false;
        };
        matches!(logical.operator, oxc_ast::ast::LogicalOperator::And)
            && matches!(
                unwrap_parenthesized(&logical.right),
                Expression::AssignmentExpression(_)
            )
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_export_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 {
            return false;
        }
        arrow
            .body
            .as_function_body()
            .and_then(|body| body.statements.first())
            .is_some_and(|stmt| matches!(stmt, Statement::ForInStatement(_)))
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_toesm_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() < 2 || arrow.params.items.len() > 3 {
            return false;
        }
        let Some(statements) = arrow.body.as_function_body().map(|body| &body.statements) else {
            return false;
        };
        statements.len() >= 3
            && statements.iter().any(|stmt| {
                let Statement::VariableDeclaration(var_decl) = stmt else {
                    return false;
                };
                var_decl
                    .declarations
                    .iter()
                    .any(|decl| matches!(&decl.init, Some(Expression::ConditionalExpression(_))))
            })
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_tocommonjs_pattern(
        arrow: &ArrowFunctionExpression<'_>,
    ) -> bool {
        if arrow.params.items.len() != 1 {
            return false;
        }
        if let Some(statements) = arrow.body.as_function_body().map(|body| &body.statements) {
            return statements.len() >= 2 && statements.iter().any(|stmt| {
                matches!(stmt, Statement::IfStatement(if_stmt) if matches!(&if_stmt.test, Expression::AssignmentExpression(_)))
            });
        }
        let Some(Expression::CallExpression(call)) = arrow.body.as_expression() else {
            return false;
        };
        call.arguments.iter().any(|arg| {
            let Some(Expression::CallExpression(inner_call)) = arg.as_expression() else {
                return false;
            };
            inner_call
                .arguments
                .get(1)
                .and_then(|arg| arg.as_expression())
                .is_some_and(|expr| matches!(expr, Expression::StringLiteral(s) if s.value.as_str() == "__esModule"))
        })
    }
}
