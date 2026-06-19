use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_super_get_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Reflect"
                && member.property.name.as_str() == "get"
            {
                return true;
            }
        }
        false
    }

    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_super_set_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Reflect"
                && member.property.name.as_str() == "set"
            {
                return true;
            }
        }
        false
    }

    /// `__pow`: `Math.pow(base, exp)`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_pow_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Math"
                && member.property.name.as_str() == "pow"
            {
                return true;
            }
        }
        false
    }

    /// `__propKey`: `typeof x === "symbol" ? x : "".concat(x)`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_prop_key_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 || body.statements.len() != 1 {
            return false;
        }
        if let Some(Statement::ReturnStatement(ret)) = body.statements.first()
            && let Some(Expression::ConditionalExpression(cond)) = &ret.argument
            && let Expression::BinaryExpression(bin) = &cond.test
            && let Expression::UnaryExpression(unary) = &bin.left
            && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
            && let Expression::StringLiteral(s) = &bin.right
            && s.value.as_str() == "symbol"
        {
            return true;
        }
        false
    }

    /// `__toBinary` / `__toBinaryNode`: Base64 to `Uint8Array` conversion
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_to_binary_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::CallExpression(call)) = &decl.init {
                        if let Expression::Identifier(ident) = &call.callee
                            && ident.name.as_str() == "atob"
                        {
                            return true;
                        }
                        if let Expression::StaticMemberExpression(member) = &call.callee
                            && let Expression::Identifier(obj) = &member.object
                            && obj.name.as_str() == "Buffer"
                            && member.property.name.as_str() == "from"
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `__earlyAccess`: `throw new ReferenceError("...")`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_early_access_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ThrowStatement(throw_stmt) = stmt
                && let Expression::NewExpression(new_expr) = &throw_stmt.argument
                && let Expression::Identifier(ident) = &new_expr.callee
                && ident.name.as_str() == "ReferenceError"
            {
                return true;
            }
        }
        false
    }

    /// `__async`: `Promise.resolve().then(...)` async lowering (esbuild)
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_async_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && member.property.name.as_str() == "then"
                && let Expression::CallExpression(inner) = &member.object
                && let Expression::StaticMemberExpression(inner_member) = &inner.callee
                && let Expression::Identifier(obj) = &inner_member.object
                && obj.name.as_str() == "Promise"
                && inner_member.property.name.as_str() == "resolve"
            {
                return true;
            }
        }
        false
    }
}
