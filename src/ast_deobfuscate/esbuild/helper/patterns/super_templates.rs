use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_super_wrapper_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ObjectExpression(obj)) = &ret.argument
            {
                let mut has_get = false;
                let mut has_set = false;
                for prop in &obj.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                        && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                    {
                        match key.name.as_str() {
                            "get" | "_ " => has_get = true,
                            "set" => has_set = true,
                            _ => {}
                        }
                    }
                }
                if has_get && has_set {
                    return true;
                }
            }
        }
        false
    }

    /// `__makeTemplateObject`: tslib template object creation
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_make_template_object_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::StaticMemberExpression(member) = &if_stmt.test
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Object"
            {
                return true;
            }
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
                && let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left
                && member.property.name.as_str() == "raw"
            {
                return true;
            }
        }
        false
    }

    /// `__setFunctionName`: Set function name property
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_set_function_name_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::BinaryExpression(bin) = &if_stmt.test
                && let Expression::UnaryExpression(unary) = &bin.left
                && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
                && let Expression::StringLiteral(s) = &bin.right
                && s.value.as_str() == "symbol"
            {
                return true;
            }
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Object"
                && member.property.name.as_str() == "defineProperty"
                && call.arguments.len() >= 2
                && let Some(arg) = call.arguments.get(1)
                && let Some(Expression::StringLiteral(s)) = arg.as_expression()
                && s.value.as_str() == "name"
            {
                return true;
            }
        }
        false
    }
}
