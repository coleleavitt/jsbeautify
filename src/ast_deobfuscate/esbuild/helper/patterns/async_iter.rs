use oxc_ast::ast::{BindingPattern, Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_await_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 || body.statements.len() > 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ConditionalExpression(cond)) = &ret.argument
                && let Expression::BinaryExpression(bin) = &cond.test
                && bin.operator == oxc_ast::ast::BinaryOperator::Instanceof
            {
                return true;
            }
        }
        false
    }

    /// `__asyncDelegator`: yield* delegation wrapper
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_async_delegator_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::ObjectExpression(obj)) = &decl.init {
                        let mut has_next = false;
                        let mut has_throw = false;
                        for prop in &obj.properties {
                            if let oxc_ast::ast::ObjectPropertyKind::SpreadProperty(_) = prop {
                                continue;
                            }
                            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                                && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                            {
                                match key.name.as_str() {
                                    "next" => has_next = true,
                                    "throw" => has_throw = true,
                                    _ => {}
                                }
                            }
                        }
                        if has_next && has_throw {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `__asyncValues`: Symbol.asyncIterator iteration
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_async_values_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::UnaryExpression(unary) = &if_stmt.test
                && unary.operator == oxc_ast::ast::UnaryOperator::LogicalNot
                && let Expression::StaticMemberExpression(member) = &unary.argument
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Symbol"
                && member.property.name.as_str() == "asyncIterator"
            {
                return true;
            }
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::CallExpression(call)) = &decl.init
                        && let Expression::ComputedMemberExpression(member) = &call.callee
                        && let Expression::StaticMemberExpression(sym) = &member.expression
                        && let Expression::Identifier(obj) = &sym.object
                        && obj.name.as_str() == "Symbol"
                        && sym.property.name.as_str() == "asyncIterator"
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `__yieldStar`: yield* in async generators
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_yield_star_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let BindingPattern::BindingIdentifier(ident) = &decl.id
                        && (ident.name.as_str() == "inner" || ident.name.as_str() == "e")
                        && let Some(Expression::CallExpression(_)) = &decl.init
                    {
                        return true;
                    }
                }
            }
            if let Statement::TryStatement(try_stmt) = stmt
                && let Some(handler) = &try_stmt.handler
                && !handler.body.body.is_empty()
            {
                return true;
            }
        }
        false
    }

    /// `__forAwait`: for-await loop lowering
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_for_await_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::TryStatement(try_stmt) = stmt {
                for inner in &try_stmt.block.body {
                    if let Statement::WhileStatement(while_stmt) = inner
                        && let Expression::UnaryExpression(unary) = &while_stmt.test
                        && unary.operator == oxc_ast::ast::UnaryOperator::LogicalNot
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}
