use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_get_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Statement::ThrowStatement(throw_stmt) = &if_stmt.consequent
                && let Expression::NewExpression(new_expr) = &throw_stmt.argument
                && let Expression::Identifier(ident) = &new_expr.callee
                && ident.name.as_str() == "TypeError"
                && let Expression::LogicalExpression(logical) = &if_stmt.test
                && let Expression::BinaryExpression(bin) = &logical.left
                && let Expression::StringLiteral(s) = &bin.right
                && (s.value.as_str() == "a" || s.value.as_str() == "m")
            {
                return true;
            }
        }
        false
    }

    /// `__privateSet`: `if (kind === "m") throw new TypeError("Private method is not writable")`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_set_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 4 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::BinaryExpression(bin) = &if_stmt.test
                && let Expression::StringLiteral(s) = &bin.right
                && s.value.as_str() == "m"
                && let Statement::ThrowStatement(throw_stmt) = &if_stmt.consequent
                && let Expression::NewExpression(new_expr) = &throw_stmt.argument
                && let Expression::Identifier(ident) = &new_expr.callee
                && ident.name.as_str() == "TypeError"
            {
                return true;
            }
        }
        false
    }

    /// `__privateAdd`: `if (state.has(obj)) throw new TypeError(...); state.add(obj)`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_add_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }

        let mut has_state_has_check = false;
        let mut has_state_add_or_set = false;

        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::CallExpression(call) = &if_stmt.test
                && let Expression::StaticMemberExpression(member) = &call.callee
                && member.property.name.as_str() == "has"
            {
                has_state_has_check = true;
            }
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::CallExpression(call) = &expr_stmt.expression
                && let Expression::StaticMemberExpression(member) = &call.callee
                && (member.property.name.as_str() == "add" || member.property.name.as_str() == "set")
            {
                has_state_add_or_set = true;
            }
        }

        has_state_has_check && has_state_add_or_set
    }

    /// `__privateIn`: `typeof state === "function" ? receiver === state : state.has(receiver)`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_in_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 || body.statements.is_empty() {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::BinaryExpression(bin) = &if_stmt.test
                && let Expression::UnaryExpression(unary) = &bin.left
                && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
            {
                return true;
            }
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ConditionalExpression(cond)) = &ret.argument
                && let Expression::BinaryExpression(bin) = &cond.test
                && let Expression::UnaryExpression(unary) = &bin.left
                && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
                && let Expression::StringLiteral(s) = &bin.right
                && s.value.as_str() == "function"
            {
                return true;
            }
        }
        false
    }
}
