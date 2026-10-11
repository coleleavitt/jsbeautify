use oxc_ast::ast::{
    BinaryOperator, BindingPattern, CallExpression, Expression, ForOfStatement, ForStatementLeft, Statement,
};

use super::descriptor::{
    argument_is_ident, call_callee_is_helper_or_object_property, expression_is_ident, expression_is_string,
    stmt_has_reexport_define_property_copy,
};
use super::string_search::{body_contains_esmodule_check, stmt_returns_ident};
use super::types::{BunHelper, BunHelperMap};

/// `__copyProps`: `(H, $, q) => { var K = __getOwnPropNames($); for (let _ of K) ... }`.
/// Exactly 3 params (simple identifiers, not destructuring), block body,
/// first statement is `VarDecl` with a `CallExpression` init, has for-of,
/// does NOT contain `__esModule` (which would indicate __toESM instead).
pub(super) fn is_copyprops_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    if !(3..=4).contains(&arrow.params.items.len()) || arrow.body.is_expression() {
        return false;
    }
    let Some(body) = arrow.body.as_function_body() else {
        return false;
    };
    // All params must be simple identifiers (not destructured)
    for p in &arrow.params.items {
        if !matches!(p.pattern, BindingPattern::BindingIdentifier(_)) {
            return false;
        }
    }

    let Some(source_param) = param_name(&arrow.params.items[1].pattern) else {
        return false;
    };
    let Some(keys_name) = reexport_keys_binding(arrow, source_param, helpers) else {
        return false;
    };
    let has_for_of_over_keys = body
        .statements
        .iter()
        .any(|stmt| matches!(stmt, Statement::ForOfStatement(for_of) if expression_is_ident(&for_of.right, keys_name)));

    // Must NOT contain __esModule (that's __toESM)
    has_for_of_over_keys && !body_contains_esmodule_check(&body.statements)
}

/// `__reExport`: Bun's 3-param export-star helper. It skips the `"default"`
/// key and conditionally copies descriptors into a second target before
/// returning the third parameter.
pub(super) fn is_reexport_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    if arrow.params.items.len() != 3 || arrow.body.is_expression() {
        return false;
    }
    let Some(body) = arrow.body.as_function_body() else {
        return false;
    };
    if !arrow
        .params
        .items
        .iter()
        .all(|p| matches!(p.pattern, BindingPattern::BindingIdentifier(_)))
    {
        return false;
    }

    let Some(target_param) = param_name(&arrow.params.items[0].pattern) else {
        return false;
    };
    let Some(source_param) = param_name(&arrow.params.items[1].pattern) else {
        return false;
    };
    let BindingPattern::BindingIdentifier(third) = &arrow.params.items[2].pattern else {
        return false;
    };
    let return_param = third.name.as_str();

    let Some(keys_name) = reexport_keys_binding(arrow, source_param, helpers) else {
        return false;
    };
    let Some(for_of) = body.statements.iter().find_map(|stmt| match stmt {
        Statement::ForOfStatement(for_of) if expression_is_ident(&for_of.right, keys_name) => Some(for_of.as_ref()),
        _ => None,
    }) else {
        return false;
    };
    let Some(loop_var) = for_of_loop_var_name(for_of) else {
        return false;
    };

    let returns_third_param = body
        .statements
        .iter()
        .any(|stmt| stmt_returns_ident(stmt, return_param));
    let has_default_skip = stmt_has_default_guard_for_loop_var(&for_of.body, loop_var);
    let has_descriptor_copy =
        stmt_has_reexport_define_property_copy(&for_of.body, target_param, source_param, loop_var, helpers);

    returns_third_param && has_default_skip && has_descriptor_copy && !body_contains_esmodule_check(&body.statements)
}

fn param_name<'a>(pattern: &'a BindingPattern<'a>) -> Option<&'a str> {
    let BindingPattern::BindingIdentifier(binding) = pattern else {
        return None;
    };
    Some(binding.name.as_str())
}

fn reexport_keys_binding<'a>(
    arrow: &'a oxc_ast::ast::ArrowFunctionExpression<'a>,
    source_param: &str,
    helpers: &BunHelperMap,
) -> Option<&'a str> {
    let Statement::VariableDeclaration(var_decl) = arrow.body.as_function_body()?.statements.first()? else {
        return None;
    };
    let decl = var_decl.declarations.first()?;
    let BindingPattern::BindingIdentifier(binding) = &decl.id else {
        return None;
    };
    let Some(Expression::CallExpression(call)) = &decl.init else {
        return None;
    };
    is_get_own_prop_names_call(call, source_param, helpers).then_some(binding.name.as_str())
}

fn is_get_own_prop_names_call(call: &CallExpression<'_>, source_param: &str, helpers: &BunHelperMap) -> bool {
    call.arguments
        .first()
        .is_some_and(|arg| argument_is_ident(arg, source_param))
        && call_callee_is_helper_or_object_property(call, helpers, BunHelper::GetOwnPropNames, "getOwnPropertyNames")
}

fn for_of_loop_var_name<'a>(for_of: &'a ForOfStatement<'a>) -> Option<&'a str> {
    match &for_of.left {
        ForStatementLeft::VariableDeclaration(var_decl) => {
            let decl = var_decl.declarations.first()?;
            param_name(&decl.id)
        }
        ForStatementLeft::AssignmentTargetIdentifier(ident) => Some(ident.name.as_str()),
        _ => None,
    }
}

fn stmt_has_default_guard_for_loop_var(stmt: &Statement<'_>, loop_var: &str) -> bool {
    match stmt {
        Statement::IfStatement(if_stmt) => {
            expr_has_default_guard_for_loop_var(&if_stmt.test, loop_var)
                || stmt_has_default_guard_for_loop_var(&if_stmt.consequent, loop_var)
                || if_stmt
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| stmt_has_default_guard_for_loop_var(alternate, loop_var))
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|stmt| stmt_has_default_guard_for_loop_var(stmt, loop_var)),
        _ => false,
    }
}

fn expr_has_default_guard_for_loop_var(expr: &Expression<'_>, loop_var: &str) -> bool {
    match expr {
        Expression::BinaryExpression(binary)
            if matches!(
                binary.operator,
                BinaryOperator::Inequality | BinaryOperator::StrictInequality
            ) =>
        {
            (expression_is_ident(&binary.left, loop_var) && expression_is_string(&binary.right, "default"))
                || (expression_is_string(&binary.left, "default") && expression_is_ident(&binary.right, loop_var))
        }
        Expression::LogicalExpression(logical) => {
            expr_has_default_guard_for_loop_var(&logical.left, loop_var)
                || expr_has_default_guard_for_loop_var(&logical.right, loop_var)
        }
        Expression::ParenthesizedExpression(paren) => expr_has_default_guard_for_loop_var(&paren.expression, loop_var),
        _ => false,
    }
}
