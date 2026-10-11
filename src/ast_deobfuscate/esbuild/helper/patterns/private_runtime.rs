use oxc_ast::ast::{BindingPattern, Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_method_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt {
                if let Some(Expression::Identifier(_)) = &ret.argument
                    && body.statements.len() == 1
                {
                    return false;
                }
                if let Some(Expression::CallExpression(call)) = &ret.argument
                    && let Expression::StaticMemberExpression(member) = &call.callee
                    && (member.property.name.as_str() == "call" || member.property.name.as_str() == "bind")
                {
                    return true;
                }
            }
        }
        false
    }

    /// `__publicField`: `Object.defineProperty(obj, key, { value, ... })`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_public_field_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt {
                if let Expression::SequenceExpression(seq) = &expr_stmt.expression {
                    for expr in &seq.expressions {
                        if let Expression::CallExpression(call) = expr
                            && let Expression::StaticMemberExpression(member) = &call.callee
                            && let Expression::Identifier(obj) = &member.object
                            && obj.name.as_str() == "Object"
                            && member.property.name.as_str() == "defineProperty"
                        {
                            return true;
                        }
                    }
                }
                if let Expression::AssignmentExpression(assign) = &expr_stmt.expression
                    && let oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(_) = &assign.left
                {
                    return true;
                }
            }
        }
        false
    }

    /// `__privateWrapper`: Private field getter/setter wrapper
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_private_wrapper_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        let mut has_get = false;
        let mut has_set = false;
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ObjectExpression(obj)) = &ret.argument
            {
                for prop in &obj.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                        && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                    {
                        match key.name.as_str() {
                            "get" => has_get = true,
                            "set" => has_set = true,
                            _ => {}
                        }
                    }
                }
            }
        }
        has_get || has_set
    }

    /// `__require`: Fallback require with Proxy
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_require_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::NewExpression(new_expr)) = &ret.argument
                && let Expression::Identifier(ident) = &new_expr.callee
                && ident.name.as_str() == "Proxy"
            {
                return true;
            }
            if let Statement::TryStatement(_) = stmt {
                return true;
            }
        }
        false
    }

    /// `__glob`: `import.meta.glob` handler
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_glob_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::ObjectExpression(_)) = &decl.init
                        && let BindingPattern::BindingIdentifier(ident) = &decl.id
                        && (ident.name.as_str().contains("glob") || ident.name.as_str().contains("modules"))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}
