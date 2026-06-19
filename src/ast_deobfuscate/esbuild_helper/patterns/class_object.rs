use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild_helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_extends_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
                && let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left
                && member.property.name.as_str() == "prototype"
                && let Expression::CallExpression(call) = &assign.right
                && let Expression::StaticMemberExpression(callee) = &call.callee
                && let Expression::Identifier(obj) = &callee.object
                && obj.name.as_str() == "Object"
                && callee.property.name.as_str() == "create"
            {
                return true;
            }
        }
        false
    }

    /// `__assign`: `Object.assign || function(t) { for (var p in s) ... }`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_assign_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.is_empty() {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForInStatement(_) = stmt {
                return true;
            }
        }
        false
    }

    /// `__rest`: `{ a, ...rest } = obj` rest extraction
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_rest_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForInStatement(for_in) = stmt
                && let Statement::IfStatement(if_stmt) = &for_in.body
                && let Expression::CallExpression(call) = &if_stmt.test
                && let Expression::StaticMemberExpression(member) = &call.callee
                && member.property.name.as_str() == "indexOf"
            {
                return true;
            }
        }
        false
    }

    /// `__spread` / `__spreadArrays` / `__spreadArray`: spread helper patterns
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_spread_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ForStatement(for_stmt) = stmt
                && matches!(&for_stmt.body, Statement::ForStatement(_))
            {
                return true;
            }
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && member.property.name.as_str() == "concat"
            {
                return true;
            }
        }
        false
    }

    /// `__values`: `Symbol.iterator` iteration
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_values_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 1 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::LogicalExpression(logical)) = &decl.init
                        && let Expression::ComputedMemberExpression(member) = &logical.right
                        && let Expression::StaticMemberExpression(sym) = &member.expression
                        && let Expression::Identifier(ident) = &sym.object
                        && ident.name.as_str() == "Symbol"
                        && sym.property.name.as_str() == "iterator"
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `__read`: `while ((n === void 0 || n-- > 0) && !(r = i.next()).done) ar.push(r.value)`
    pub(in crate::ast_deobfuscate::esbuild_helper) fn is_read_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.is_empty() {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::WhileStatement(while_stmt) = stmt
                && let Expression::LogicalExpression(_) = &while_stmt.test
            {
                return true;
            }
            if let Statement::TryStatement(_) = stmt {
                return true;
            }
        }
        false
    }
}
