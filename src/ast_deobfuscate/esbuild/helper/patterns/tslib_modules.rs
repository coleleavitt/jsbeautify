use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

pub(in crate::ast_deobfuscate::esbuild::helper) fn check_for_set_module_default(stmts: &[Statement<'_>]) -> bool {
    for stmt in stmts {
        if let Statement::ExpressionStatement(expr_stmt) = stmt
            && let Expression::CallExpression(call) = &expr_stmt.expression
            && let Expression::Identifier(ident) = &call.callee
            && (ident.name.as_str().contains("setModuleDefault") || ident.name.as_str().contains("SetModuleDefault"))
        {
            return true;
        }
    }
    false
}

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_import_default_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 || body.statements.len() != 1 {
            return false;
        }
        let Some(Statement::ReturnStatement(ret)) = body.statements.first() else {
            return false;
        };
        let Some(ret_arg) = &ret.argument else { return false };

        let cond = match ret_arg {
            Expression::ConditionalExpression(c) => c,
            Expression::ParenthesizedExpression(p) => {
                if let Expression::ConditionalExpression(c) = &p.expression {
                    c
                } else {
                    return false;
                }
            }
            _ => return false,
        };

        let test = match &cond.test {
            Expression::ParenthesizedExpression(p) => &p.expression,
            other => other,
        };

        if let Expression::LogicalExpression(logical) = test
            && let Expression::StaticMemberExpression(member) = &logical.right
            && member.property.name.as_str() == "__esModule"
        {
            return true;
        }
        false
    }

    /// `__importStar`: checks `mod.__esModule`, calls `__setModuleDefault`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_import_star_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 || body.statements.len() < 2 {
            return false;
        }

        let mut has_esmodule_check = false;

        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &if_stmt.test
                && let Expression::StaticMemberExpression(member) = &logical.right
                && member.property.name.as_str() == "__esModule"
            {
                has_esmodule_check = true;
            }
        }
        let has_set_module_default = check_for_set_module_default(&body.statements);
        has_esmodule_check && has_set_module_default
    }

    /// `__createBinding`: `Object.defineProperty(o, k2, { enumerable: true, get: ... })`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_create_binding_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::CallExpression(call) = &expr_stmt.expression
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Object"
                && member.property.name.as_str() == "defineProperty"
                && call.arguments.len() >= 3
                && let Some(arg) = call.arguments.get(2)
                && let Some(Expression::ObjectExpression(obj_expr)) = arg.as_expression()
            {
                for prop in &obj_expr.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                        && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                        && key.name.as_str() == "enumerable"
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `__exportStar`: `for (var p in m) if (p !== "default") __createBinding(...)`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_export_star_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::ForInStatement(for_in) = stmt
                && let Statement::IfStatement(if_stmt) = &for_in.body
            {
                if let Expression::BinaryExpression(bin) = &if_stmt.test
                    && let Expression::StringLiteral(s) = &bin.right
                    && s.value.as_str() == "default"
                {
                    return true;
                }
                if let Expression::LogicalExpression(logical) = &if_stmt.test
                    && let Expression::BinaryExpression(bin) = &logical.left
                    && let Expression::StringLiteral(s) = &bin.right
                    && s.value.as_str() == "default"
                {
                    return true;
                }
            }
        }
        false
    }
}
