use oxc_ast::ast::{Argument, ArrowFunctionBody, CallExpression, Expression, ObjectPropertyKind, Statement};

use super::types::{BunHelper, BunHelperMap};

pub(super) fn stmt_has_reexport_define_property_copy(
    stmt: &Statement<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            expr_has_reexport_define_property_copy(&expr_stmt.expression, target_param, source_param, loop_var, helpers)
        }
        Statement::IfStatement(if_stmt) => {
            stmt_has_reexport_define_property_copy(&if_stmt.consequent, target_param, source_param, loop_var, helpers)
                || if_stmt.alternate.as_ref().is_some_and(|alternate| {
                    stmt_has_reexport_define_property_copy(alternate, target_param, source_param, loop_var, helpers)
                })
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_has_reexport_define_property_copy(stmt, target_param, source_param, loop_var, helpers)),
        _ => false,
    }
}

fn expr_has_reexport_define_property_copy(
    expr: &Expression<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    match expr {
        Expression::CallExpression(call) => {
            is_define_property_copy_call(call, target_param, source_param, loop_var, helpers)
        }
        Expression::ParenthesizedExpression(paren) => {
            expr_has_reexport_define_property_copy(&paren.expression, target_param, source_param, loop_var, helpers)
        }
        Expression::SequenceExpression(seq) => seq
            .expressions
            .iter()
            .any(|expr| expr_has_reexport_define_property_copy(expr, target_param, source_param, loop_var, helpers)),
        Expression::LogicalExpression(logical) => {
            expr_has_reexport_define_property_copy(&logical.left, target_param, source_param, loop_var, helpers)
                || expr_has_reexport_define_property_copy(&logical.right, target_param, source_param, loop_var, helpers)
        }
        _ => false,
    }
}

fn is_define_property_copy_call(
    call: &CallExpression<'_>,
    target_param: &str,
    source_param: &str,
    loop_var: &str,
    helpers: &BunHelperMap,
) -> bool {
    if !call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProp, "defineProperty") {
        return false;
    }
    let Some(target_arg) = call.arguments.first() else {
        return false;
    };
    let Some(key_arg) = call.arguments.get(1) else {
        return false;
    };
    let Some(descriptor_arg) = call.arguments.get(2) else {
        return false;
    };

    argument_is_ident(target_arg, target_param)
        && argument_is_ident(key_arg, loop_var)
        && argument_descriptor_getter_reads_source_key(descriptor_arg, source_param, loop_var)
}

pub(super) fn call_callee_is_helper_or_object_property(
    call: &CallExpression<'_>,
    helpers: &BunHelperMap,
    helper: BunHelper,
    object_property: &str,
) -> bool {
    match &call.callee {
        Expression::Identifier(id) => {
            id.name.as_str() == helper.canonical()
                || helpers
                    .get(id.name.as_str())
                    .is_some_and(|detected_helper| *detected_helper == helper)
        }
        Expression::StaticMemberExpression(member) => {
            matches!(&member.object, Expression::Identifier(obj) if obj.name.as_str() == "Object")
                && member.property.name.as_str() == object_property
        }
        _ => false,
    }
}

fn argument_descriptor_getter_reads_source_key(arg: &Argument<'_>, source_param: &str, loop_var: &str) -> bool {
    let Argument::ObjectExpression(obj) = arg else {
        return false;
    };
    obj.properties.iter().any(|property| {
        let ObjectPropertyKind::ObjectProperty(prop) = property else {
            return false;
        };
        prop.key.name().is_some_and(|name| name.as_ref() == "get")
            && expr_contains_source_key(&prop.value, source_param, loop_var)
    })
}

fn argument_contains_source_key(arg: &Argument<'_>, source_param: &str, loop_var: &str) -> bool {
    match arg {
        Argument::Identifier(id) => id.name.as_str() == source_param,
        Argument::ComputedMemberExpression(member) => {
            expression_is_ident(&member.object, source_param) && expression_is_ident(&member.expression, loop_var)
        }
        Argument::ObjectExpression(obj) => obj.properties.iter().any(|property| {
            let ObjectPropertyKind::ObjectProperty(prop) = property else {
                return false;
            };
            expr_contains_source_key(&prop.value, source_param, loop_var)
        }),
        _ => false,
    }
}

fn expr_contains_source_key(expr: &Expression<'_>, source_param: &str, loop_var: &str) -> bool {
    match expr {
        Expression::ComputedMemberExpression(member) => {
            expression_is_ident(&member.object, source_param) && expression_is_ident(&member.expression, loop_var)
        }
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) => body
                .statements
                .iter()
                .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var)),
            body => body
                .as_expression()
                .is_some_and(|expr| expr_contains_source_key(expr, source_param, loop_var)),
        },
        Expression::FunctionExpression(func) => func.body.as_ref().is_some_and(|body| {
            body.statements
                .iter()
                .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var))
        }),
        Expression::ParenthesizedExpression(paren) => {
            expr_contains_source_key(&paren.expression, source_param, loop_var)
        }
        Expression::CallExpression(call) => {
            expr_contains_source_key(&call.callee, source_param, loop_var)
                || call
                    .arguments
                    .iter()
                    .any(|arg| argument_contains_source_key(arg, source_param, loop_var))
        }
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|property| {
            let ObjectPropertyKind::ObjectProperty(prop) = property else {
                return false;
            };
            expr_contains_source_key(&prop.value, source_param, loop_var)
        }),
        Expression::BinaryExpression(binary) => {
            expr_contains_source_key(&binary.left, source_param, loop_var)
                || expr_contains_source_key(&binary.right, source_param, loop_var)
        }
        Expression::LogicalExpression(logical) => {
            expr_contains_source_key(&logical.left, source_param, loop_var)
                || expr_contains_source_key(&logical.right, source_param, loop_var)
        }
        Expression::ConditionalExpression(cond) => {
            expr_contains_source_key(&cond.test, source_param, loop_var)
                || expr_contains_source_key(&cond.consequent, source_param, loop_var)
                || expr_contains_source_key(&cond.alternate, source_param, loop_var)
        }
        _ => false,
    }
}

fn stmt_contains_source_key(stmt: &Statement<'_>, source_param: &str, loop_var: &str) -> bool {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            expr_contains_source_key(&expr_stmt.expression, source_param, loop_var)
        }
        Statement::ReturnStatement(ret) => ret
            .argument
            .as_ref()
            .is_some_and(|arg| expr_contains_source_key(arg, source_param, loop_var)),
        Statement::IfStatement(if_stmt) => {
            expr_contains_source_key(&if_stmt.test, source_param, loop_var)
                || stmt_contains_source_key(&if_stmt.consequent, source_param, loop_var)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_contains_source_key(alternate, source_param, loop_var))
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_contains_source_key(stmt, source_param, loop_var)),
        _ => false,
    }
}

pub(super) fn argument_is_ident(arg: &Argument<'_>, name: &str) -> bool {
    matches!(arg, Argument::Identifier(id) if id.name.as_str() == name)
}

pub(super) fn expression_is_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name.as_str() == name)
}

pub(super) fn expression_is_string(expr: &Expression<'_>, value: &str) -> bool {
    matches!(expr, Expression::StringLiteral(lit) if lit.value.as_str() == value)
}
