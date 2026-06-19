use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

pub(in crate::ast_deobfuscate::esbuild_helper) fn contains_symbol_dispose(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::StaticMemberExpression(member) => {
            if let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Symbol"
                && (member.property.name.as_str() == "dispose" || member.property.name.as_str() == "asyncDispose")
            {
                return true;
            }
            false
        }
        Expression::CallExpression(call) => {
            if let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Symbol"
                && member.property.name.as_str() == "for"
                && let Some(arg) = call.arguments.first()
                && let Some(Expression::StringLiteral(s)) = arg.as_expression()
                && s.value.as_str().contains("dispose")
            {
                return true;
            }
            false
        }
        _ => false,
    }
}

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_add_disposable_resource_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(init) = &decl.init {
                        if contains_symbol_dispose(init) {
                            return true;
                        }
                        if let Expression::LogicalExpression(logical) = init
                            && (contains_symbol_dispose(&logical.left) || contains_symbol_dispose(&logical.right))
                        {
                            return true;
                        }
                    }
                }
            }
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::Identifier(ident) = &if_stmt.test
                && ident.name.as_str().contains("async")
            {
                return true;
            }
        }
        false
    }

    /// `__disposeResources`: `SuppressedError` constructor, resource stack
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_dispose_resources_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::ConditionalExpression(cond)) = &decl.init
                        && let Expression::BinaryExpression(bin) = &cond.test
                        && let Expression::UnaryExpression(unary) = &bin.left
                        && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
                        && let Expression::Identifier(ident) = &unary.argument
                        && ident.name.as_str() == "SuppressedError"
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}
