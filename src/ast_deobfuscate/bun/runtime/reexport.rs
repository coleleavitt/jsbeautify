use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, CallExpression, Expression, ForOfStatement, ForStatementLeft,
    ObjectPropertyKind, Statement,
};

use super::ast_utils::{arrow_contains, block_body, expression_body, expression_contains, is_ident, param_names};
use super::descriptor::{
    argument_is_ident, call_callee_is_helper, call_callee_is_helper_or_object_property, expression_is_ident,
    expression_is_string, stmt_has_reexport_define_property_copy,
};
use super::string_search::{body_contains_esmodule_check, stmt_returns_ident};
use super::types::{BunHelper, BunHelperMap};

/// `__copyProps`, Bun's or esbuild's.
pub(super) fn is_copyprops_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    is_bun_copyprops_helper(arrow, helpers) || is_esbuild_copyprops_helper(arrow, helpers)
}

/// Bun's `__copyProps`: `(H, $, q) => { var K = __getOwnPropNames($); for (let _ of K) ... }`.
/// Exactly 3 params (simple identifiers, not destructuring), block body,
/// first statement is `VarDecl` with a `CallExpression` init, has for-of,
/// does NOT contain `__esModule` (which would indicate __toESM instead).
fn is_bun_copyprops_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
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

/// esbuild's `__copyProps`, which defines a getter on `to` for each own key of
/// `from` (a `for…of` loop, or an indexed loop with `.bind` getters below ES6):
///
/// ```js
/// (to, from, except, desc) => {
///   if (from && typeof from === "object" || typeof from === "function")
///     for (let key of __getOwnPropNames(from))
///       if (!__hasOwnProp.call(to, key) && key !== except)
///         __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
///   return to;
/// }
/// ```
fn is_esbuild_copyprops_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    let Some(params) = param_names(arrow) else {
        return false;
    };
    let (Some(body), 3..=4) = (block_body(arrow), params.len()) else {
        return false;
    };
    let (to, from) = (params[0], params[1]);
    let returns_to = body.statements.last().is_some_and(|stmt| stmt_returns_ident(stmt, to));
    returns_to
        && arrow_contains(arrow, |expr| {
            matches!(expr, Expression::CallExpression(call)
                if call_callee_is_helper_or_object_property(call, helpers, BunHelper::GetOwnPropNames, "getOwnPropertyNames")
                    && call.arguments.first().is_some_and(|arg| argument_is_ident(arg, from)))
        })
        && arrow_contains(
            arrow,
            |expr| matches!(expr, Expression::CallExpression(call) if is_getter_copy(call, helpers, to, from)),
        )
}

/// `__defProp(to, key, { get: … from[…] …, … })`.
fn is_getter_copy(call: &CallExpression<'_>, helpers: &BunHelperMap, to: &str, from: &str) -> bool {
    let [target, _, Argument::ObjectExpression(descriptor)] = call.arguments.as_slice() else {
        return false;
    };
    let reads_from = |expr: &Expression<'_>| {
        expression_contains(
            expr,
            |read| matches!(read, Expression::ComputedMemberExpression(member) if is_ident(&member.object, from)),
        )
    };
    call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProp, "defineProperty")
        && argument_is_ident(target, to)
        && descriptor.properties.iter().any(|property| {
            matches!(property, ObjectPropertyKind::ObjectProperty(prop)
                if prop.key.static_name().is_some_and(|key| key == "get") && reads_from(&prop.value))
        })
}

/// `__reExport`: esbuild's `(target, mod, secondTarget) => (__copyProps(target, mod, "default"),
/// secondTarget && __copyProps(secondTarget, mod, "default"))`, or Bun's
/// block-bodied helper below.
pub(super) fn is_reexport_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    is_esbuild_reexport_helper(arrow, helpers) || is_bun_reexport_helper(arrow, helpers)
}

fn is_esbuild_reexport_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    let Some(&[target, module, _]) = param_names(arrow).as_deref() else {
        return false;
    };
    let Some(Expression::SequenceExpression(seq)) = expression_body(arrow) else {
        return false;
    };
    let [Expression::CallExpression(copy), Expression::LogicalExpression(_)] = seq.expressions.as_slice() else {
        return false;
    };
    call_callee_is_helper(copy, helpers, BunHelper::CopyProps)
        && matches!(copy.arguments.as_slice(), [t, m, Argument::StringLiteral(except)]
            if argument_is_ident(t, target) && argument_is_ident(m, module) && except.value == "default")
}

/// Bun's 3-param export-star helper. It skips the `"default"`
/// key and conditionally copies descriptors into a second target before
/// returning the third parameter.
fn is_bun_reexport_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
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
