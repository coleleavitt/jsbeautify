use oxc_ast::ast::{ArrowFunctionExpression, Expression, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_commonjs_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 || !arrow.expression {
            return false;
        }

        let Some(body) = &arrow.body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = body else {
            return false;
        };

        let Expression::ArrowFunctionExpression(inner_arrow) = &expr_stmt.expression else {
            return false;
        };

        if !inner_arrow.params.items.is_empty() || !inner_arrow.expression {
            return false;
        }

        let Some(inner_body) = inner_arrow.body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(_inner_expr) = inner_body else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = body else {
            eprintln!("[CJS DEBUG] Failed: not ExpressionStatement");
            return false;
        };

        let Expression::ArrowFunctionExpression(inner_arrow) = &expr_stmt.expression else {
            eprintln!("[CJS DEBUG] Failed: inner expression is not ArrowFunctionExpression");
            return false;
        };

        eprintln!(
            "[CJS DEBUG] Found inner arrow, params: {}, expression: {}",
            inner_arrow.params.items.len(),
            inner_arrow.expression
        );

        if !inner_arrow.params.items.is_empty() || !inner_arrow.expression {
            eprintln!("[CJS DEBUG] Failed: inner params or expression check");
            return false;
        }

        eprintln!(
            "[CJS DEBUG] Inner body statements: {}",
            inner_arrow.body.statements.len()
        );

        let Some(inner_body) = inner_arrow.body.statements.first() else {
            eprintln!("[CJS DEBUG] Failed: no inner body statements");
            return false;
        };

        let Statement::ExpressionStatement(inner_expr) = inner_body else {
            eprintln!("[CJS DEBUG] Failed: inner body not expression statement");
            return false;
        };

        let inner_expr_unwrapped = match &inner_expr.expression {
            Expression::ParenthesizedExpression(paren) => &paren.expression,
            other => other,
        };

        if let Expression::SequenceExpression(seq) = inner_expr_unwrapped
            && seq.expressions.len() == 2
            && let Expression::StaticMemberExpression(member) = &seq.expressions[1]
            && member.property.name.as_str() == "exports"
        {
            return true;
        }

        false
    }

    /// Pattern: `(fn, res) => () => (fn && (res = fn(fn = 0)), res)`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_esm_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 || !arrow.expression {
            return false;
        }

        let Some(body) = &arrow.body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = body else {
            return false;
        };

        let Expression::ArrowFunctionExpression(inner_arrow) = &expr_stmt.expression else {
            return false;
        };

        if !inner_arrow.params.items.is_empty() || !inner_arrow.expression {
            return false;
        }

        let Some(inner_body) = inner_arrow.body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(inner_expr) = inner_body else {
            return false;
        };

        let inner_expr_unwrapped = match &inner_expr.expression {
            Expression::ParenthesizedExpression(paren) => &paren.expression,
            other => other,
        };

        if let Expression::SequenceExpression(seq) = inner_expr_unwrapped
            && seq.expressions.len() == 2
            && let Expression::LogicalExpression(logical) = &seq.expressions[0]
            && matches!(logical.operator, oxc_ast::ast::LogicalOperator::And)
        {
            if let Expression::ParenthesizedExpression(paren) = &logical.right
                && let Expression::AssignmentExpression(_) = &paren.expression
            {
                return true;
            }
            if let Expression::AssignmentExpression(_) = &logical.right {
                return true;
            }
        }

        false
    }

    /// Pattern: `for (name in all) __defProp(target, name, {get: all[name], enumerable: true})`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_export_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() != 2 || arrow.expression {
            return false;
        }

        let Some(body) = &arrow.body.statements.first() else {
            return false;
        };

        matches!(body, Statement::ForInStatement(_))
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_toesm_pattern(arrow: &ArrowFunctionExpression<'_>) -> bool {
        if arrow.params.items.len() < 2 || arrow.params.items.len() > 3 || arrow.expression {
            return false;
        }

        let statements = &arrow.body.statements;
        if statements.len() < 3 {
            return false;
        }

        for stmt in statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(init) = &decl.init
                        && let Expression::ConditionalExpression(_) = init
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_tocommonjs_pattern(
        arrow: &ArrowFunctionExpression<'_>,
    ) -> bool {
        if arrow.params.items.len() != 1 {
            return false;
        }

        if !arrow.expression {
            let statements = &arrow.body.statements;
            if statements.len() >= 2 {
                for stmt in statements {
                    if let Statement::IfStatement(if_stmt) = stmt
                        && let Expression::AssignmentExpression(_) = &if_stmt.test
                    {
                        return true;
                    }
                }
            }
            return false;
        }

        let Some(body) = &arrow.body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = body else {
            return false;
        };

        if let Expression::CallExpression(call) = &expr_stmt.expression {
            for arg in &call.arguments {
                if let Some(expr) = arg.as_expression()
                    && let Expression::CallExpression(inner_call) = expr
                    && inner_call.arguments.len() >= 2
                    && let Some(arg1) = inner_call.arguments.get(1)
                    && let Some(Expression::StringLiteral(s)) = arg1.as_expression()
                    && s.value.as_str() == "__esModule"
                {
                    return true;
                }
            }
        }

        false
    }
}
