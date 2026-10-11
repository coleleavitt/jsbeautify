use oxc_ast::ast::{ArrowFunctionExpression, BindingPattern, Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_identity_pattern(
        arrow: &ArrowFunctionExpression<'_>,
    ) -> bool {
        if arrow.params.items.len() != 1 || !arrow.body.is_expression() {
            return false;
        }

        let param_name = match arrow.params.items.first() {
            Some(param) => match &param.pattern {
                BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                _ => return false,
            },
            None => return false,
        };

        if let Some(Expression::Identifier(ident)) = arrow.body.as_expression() {
            return ident.name.as_str() == param_name;
        }

        false
    }

    /// Detect getter bind pattern: `function(q) { return this[q] }`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_getter_bind_function(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else {
            return false;
        };

        if func.params.items.len() != 1 || body.statements.len() != 1 {
            return false;
        }

        let param_name = match func.params.items.first() {
            Some(param) => match &param.pattern {
                BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                _ => return false,
            },
            None => return false,
        };

        let Some(stmt) = body.statements.first() else {
            return false;
        };

        let Statement::ReturnStatement(ret) = stmt else {
            return false;
        };

        let Some(arg) = &ret.argument else {
            return false;
        };

        if let Expression::ComputedMemberExpression(member) = arg
            && let Expression::ThisExpression(_) = &member.object
            && let Expression::Identifier(ident) = &member.expression
        {
            return ident.name.as_str() == param_name;
        }

        false
    }

    /// Detect setter bind pattern: `function(q, K) { this[q] = ... }`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_setter_bind_function(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else {
            return false;
        };

        if func.params.items.len() != 2 || body.statements.is_empty() {
            return false;
        }

        let param_name = match func.params.items.first() {
            Some(param) => match &param.pattern {
                BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                _ => return false,
            },
            None => return false,
        };

        let Some(stmt) = body.statements.first() else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = stmt else {
            return false;
        };

        if let Expression::AssignmentExpression(assign) = &expr_stmt.expression
            && let oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) = &assign.left
            && let Expression::ThisExpression(_) = &member.object
            && let Expression::Identifier(ident) = &member.expression
        {
            return ident.name.as_str() == param_name;
        }

        false
    }

    /// Detect Anthropic's modified __export with configurable:true and set: accessor
    pub(in crate::ast_deobfuscate::esbuild::helper) fn has_anthropic_export_modifications(
        arrow: &ArrowFunctionExpression<'_>,
    ) -> bool {
        if !Self::is_export_pattern(arrow) {
            return false;
        }

        let Some(body) = arrow.body.as_function_body().and_then(|body| body.statements.first()) else {
            return false;
        };

        let Statement::ForInStatement(for_in) = body else {
            return false;
        };

        let Statement::ExpressionStatement(expr_stmt) = &for_in.body else {
            return false;
        };

        let Expression::CallExpression(call) = &expr_stmt.expression else {
            return false;
        };

        if call.arguments.len() < 3 {
            return false;
        }

        let Some(obj_arg) = call.arguments.get(2) else {
            return false;
        };

        let Some(Expression::ObjectExpression(obj)) = obj_arg.as_expression() else {
            return false;
        };

        let mut has_configurable = false;
        let mut has_set = false;

        for prop in &obj.properties {
            if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &p.key
            {
                match key.name.as_str() {
                    "configurable" => {
                        if let Expression::BooleanLiteral(b) = &p.value {
                            has_configurable = b.value;
                        }
                    }
                    "set" => has_set = true,
                    _ => {}
                }
            }
        }

        has_configurable && has_set
    }
}
