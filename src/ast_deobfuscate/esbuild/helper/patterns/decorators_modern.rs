use oxc_ast::ast::{BindingPattern, Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_es_decorate_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(Expression::ObjectExpression(obj)) = &decl.init {
                        for prop in &obj.properties {
                            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                                && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
                                && key.name.as_str() == "addInitializer"
                            {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// `__decorateClass`: esbuild class decorator application
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorate_class_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForStatement(for_stmt) = stmt
                && let Statement::BlockStatement(block) = &for_stmt.body
            {
                for inner in &block.body {
                    if let Statement::VariableDeclaration(var_decl) = inner {
                        for decl in &var_decl.declarations {
                            if let BindingPattern::BindingIdentifier(ident) = &decl.id
                                && (ident.name.as_str() == "desc" || ident.name.as_str() == "decorator")
                            {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// `__decorateParam`: esbuild parameter decorator wrapper
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorate_param_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::FunctionExpression(inner_func)) = &ret.argument
                && let Some(inner_body) = &inner_func.body
            {
                for inner_stmt in &inner_body.statements {
                    if let Statement::ExpressionStatement(expr_stmt) = inner_stmt
                        && let Expression::CallExpression(_) = &expr_stmt.expression
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `__decoratorStart`: Decorator metadata initialization
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorator_start_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::ArrayExpression(arr)) = &ret.argument
                && (arr.elements.is_empty() || arr.elements.len() <= 3)
            {
                return true;
            }
        }
        false
    }

    /// `__decoratorMetadata`: Attach metadata to decorated element
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorator_metadata_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt
                && let Expression::CallExpression(call) = &expr_stmt.expression
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(obj) = &member.object
                && obj.name.as_str() == "Object"
                && member.property.name.as_str() == "defineProperty"
                && call.arguments.len() >= 2
                && let Some(arg) = call.arguments.get(1)
                && let Some(Expression::StaticMemberExpression(sym_member)) = arg.as_expression()
                && let Expression::Identifier(sym_obj) = &sym_member.object
                && sym_obj.name.as_str() == "Symbol"
            {
                return true;
            }
        }
        false
    }

    /// `__runInitializers`: Execute decorator initializers
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_run_initializers_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForStatement(for_stmt) = stmt
                && let Statement::BlockStatement(block) = &for_stmt.body
            {
                for inner in &block.body {
                    if let Statement::ExpressionStatement(expr_stmt) = inner
                        && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
                    {
                        if let Expression::ConditionalExpression(_) = &assign.right {
                            return true;
                        }
                        if let Expression::CallExpression(call) = &assign.right
                            && let Expression::StaticMemberExpression(member) = &call.callee
                            && member.property.name.as_str() == "call"
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `__decorateElement`: JS decorator element processing
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorate_element_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 3 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::SwitchStatement(switch_stmt) = stmt
                && switch_stmt.cases.len() >= 3
            {
                return true;
            }
        }
        false
    }

    /// `__copyProps`: Property copier for re-exports
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_copy_props_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForStatement(for_stmt) = stmt
                && let Statement::ExpressionStatement(expr_stmt) = &for_stmt.body
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && logical.operator == oxc_ast::ast::LogicalOperator::Or
            {
                return true;
            }
        }
        false
    }

    /// `__reExport`: export * from handler
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_re_export_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForInStatement(for_in) = stmt
                && let Statement::ExpressionStatement(expr_stmt) = &for_in.body
                && let Expression::LogicalExpression(logical) = &expr_stmt.expression
                && logical.operator == oxc_ast::ast::LogicalOperator::Or
            {
                return true;
            }
        }
        false
    }
}
