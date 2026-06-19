use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_rest_key_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 || body.statements.len() != 1 {
            return false;
        }
        if let Some(Statement::ReturnStatement(ret)) = body.statements.first()
            && let Some(Expression::BinaryExpression(bin)) = &ret.argument
            && bin.operator == oxc_ast::ast::BinaryOperator::Addition
            && let Expression::StringLiteral(_) = &bin.left
        {
            return true;
        }
        false
    }

    /// `__objRest`: Object rest extraction
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_obj_rest_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForInStatement(for_in) = stmt
                && let Statement::IfStatement(if_stmt) = &for_in.body
                && let Expression::UnaryExpression(unary) = &if_stmt.test
                && unary.operator == oxc_ast::ast::UnaryOperator::LogicalNot
                && let Expression::CallExpression(call) = &unary.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && (member.property.name.as_str() == "includes" || member.property.name.as_str() == "indexOf")
            {
                return true;
            }
        }
        false
    }

    /// `__spreadValues`: Spread with symbol support
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_spread_values_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForInStatement(_) = stmt {
                for inner in &body.statements {
                    if let Statement::IfStatement(if_stmt) = inner
                        && let Expression::Identifier(ident) = &if_stmt.test
                        && (ident.name.as_str().contains("getOwnPropertySymbols")
                            || ident.name.as_str().contains("Symbols"))
                    {
                        return true;
                    }
                }
                return true;
            }
        }
        false
    }

    /// `__spreadProps`: Spread via defineProperties
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_spread_props_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Object"
                && member.property.name.as_str() == "defineProperties"
            {
                return true;
            }
        }
        false
    }
}
