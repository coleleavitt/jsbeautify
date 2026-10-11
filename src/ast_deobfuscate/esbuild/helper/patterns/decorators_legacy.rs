use oxc_ast::ast::{Expression, Function, Statement};

use crate::ast_deobfuscate::esbuild::helper::EsbuildHelperCollector;

impl EsbuildHelperCollector {
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_decorate_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ForStatement(for_stmt) = stmt
                && let Statement::ExpressionStatement(expr_stmt) = &for_stmt.body
                && let Expression::AssignmentExpression(_) = &expr_stmt.expression
            {
                return true;
            }
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for decl in &var_decl.declarations {
                    if let Some(init) = &decl.init
                        && let Expression::ConditionalExpression(cond) = init
                        && let Expression::BinaryExpression(bin) = &cond.test
                        && let Expression::StaticMemberExpression(member) = &bin.left
                        && member.property.name.as_str() == "length"
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `__param`: `function(paramIndex, decorator) { return (target, key) => decorator(...) }`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_param_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 || body.statements.len() != 1 {
            return false;
        }
        if let Some(Statement::ReturnStatement(ret)) = body.statements.first()
            && let Some(Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)) = &ret.argument
        {
            return true;
        }
        false
    }

    /// `__metadata`: `Reflect.metadata(metadataKey, metadataValue)`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_metadata_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::LogicalExpression(logical) = &if_stmt.test
                && let Expression::BinaryExpression(bin) = &logical.left
                && let Expression::UnaryExpression(unary) = &bin.left
                && unary.operator == oxc_ast::ast::UnaryOperator::Typeof
                && let Expression::Identifier(ident) = &unary.argument
                && ident.name.as_str() == "Reflect"
            {
                return true;
            }
            if let Statement::ReturnStatement(ret) = stmt
                && let Some(Expression::CallExpression(call)) = &ret.argument
                && let Expression::StaticMemberExpression(member) = &call.callee
                && let Expression::Identifier(ident) = &member.object
                && ident.name.as_str() == "Reflect"
                && member.property.name.as_str() == "metadata"
            {
                return true;
            }
        }
        false
    }

    /// `__template`: `Object.defineProperty(cooked, "raw", { value: raw })`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_template_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() != 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Statement::BlockStatement(block) = &if_stmt.consequent
            {
                for inner in &block.body {
                    if let Statement::ExpressionStatement(expr_stmt) = inner
                        && let Expression::CallExpression(call) = &expr_stmt.expression
                        && let Expression::StaticMemberExpression(member) = &call.callee
                        && let Expression::Identifier(obj) = &member.object
                        && obj.name.as_str() == "Object"
                        && member.property.name.as_str() == "defineProperty"
                    {
                        return true;
                    }
                }
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

    /// `__name`: `Object.defineProperty(f, "name", { value: name, configurable: true })`
    pub(in crate::ast_deobfuscate::esbuild::helper) fn is_name_pattern(func: &Function<'_>) -> bool {
        let Some(body) = &func.body else { return false };
        if func.params.items.len() < 2 {
            return false;
        }
        for stmt in &body.statements {
            if let Statement::ReturnStatement(ret) = stmt {
                let arg = match &ret.argument {
                    Some(Expression::ParenthesizedExpression(paren)) => Some(&paren.expression),
                    other => other.as_ref(),
                };
                if let Some(Expression::SequenceExpression(seq)) = arg {
                    for expr in &seq.expressions {
                        if let Expression::CallExpression(call) = expr
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
                }
            }
        }
        false
    }
}
