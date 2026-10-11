use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

pub(in crate::ast_deobfuscate::esbuild::helper) fn contains_promise_pattern(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NewExpression(new_expr) => {
            let callee = match &new_expr.callee {
                Expression::ParenthesizedExpression(paren) => &paren.expression,
                other => other,
            };
            if let Expression::LogicalExpression(logical) = callee {
                let right = match &logical.right {
                    Expression::ParenthesizedExpression(paren) => &paren.expression,
                    other => other,
                };
                if let Expression::AssignmentExpression(assign) = right
                    && let Expression::Identifier(ident) = &assign.right
                    && ident.name.as_str() == "Promise"
                {
                    return true;
                }
                if let Expression::Identifier(ident) = &logical.left
                    && ident.name.as_str() == "Promise"
                {
                    return true;
                }
            }
            if let Expression::Identifier(ident) = callee
                && ident.name.as_str() == "Promise"
            {
                return true;
            }
            false
        }
        _ => false,
    }
}

pub(in crate::ast_deobfuscate::esbuild::helper) fn check_for_async_iterator(stmts: &[Statement<'_>]) -> bool {
    for stmt in stmts {
        if let Statement::ExpressionStatement(expr_stmt) = stmt
            && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
            && let oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) = &assign.left
            && let Expression::StaticMemberExpression(sym_member) = &member.expression
            && let Expression::Identifier(ident) = &sym_member.object
            && ident.name.as_str() == "Symbol"
            && sym_member.property.name.as_str() == "asyncIterator"
        {
            return true;
        }
    }
    false
}

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_awaiter_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(expr) = &ret.argument
                && contains_promise_pattern(expr)
            {
                return true;
            }
        }
        false
    }

    /// `__generator`: `{ label: 0, sent: function() {}, trys: [], ops: [] }`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_generator_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }

        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ObjectExpression(obj)) = &ret.argument
            {
                let mut has_label = false;
                let mut has_ops = false;

                for prop in &obj.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                        && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                    {
                        match key.name.as_str() {
                            "label" => has_label = true,
                            "ops" => has_ops = true,
                            _ => {}
                        }
                    }
                }

                if has_label && has_ops {
                    return true;
                }
            }
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::ObjectExpression(obj)) = &decl.init {
                        let mut has_label = false;
                        let mut has_ops = false;

                        for prop in &obj.properties {
                            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                                && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                            {
                                match key.name.as_str() {
                                    "label" => has_label = true,
                                    "ops" => has_ops = true,
                                    _ => {}
                                }
                            }
                        }

                        if has_label && has_ops {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `__asyncGenerator`: `Symbol.asyncIterator` with queue management
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_async_generator_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        check_for_async_iterator(&body.statements)
    }
}
