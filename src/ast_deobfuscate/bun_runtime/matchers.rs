use oxc_ast::ast::Expression;

use super::string_search::{
    body_calls_define_property_with_name, body_contains_esmodule_check, body_contains_esmodule_string,
};

/// `__esm`: `(H, $) => () => (H && ($ = H(H = 0)), $)` (plus paren variants).
pub(super) fn is_esm_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    if arrow.params.items.len() != 2 || !arrow.body.is_expression() {
        return false;
    }
    let Some(inner) = inner_arrow_body_expr(arrow) else {
        return false;
    };
    // inner is the returned expression after paren-unwrap.
    let Expression::SequenceExpression(seq) = inner else {
        return false;
    };
    if seq.expressions.len() != 2 {
        return false;
    }
    // [0] = LogicalExpression with && operator
    let Expression::LogicalExpression(logical) = &seq.expressions[0] else {
        return false;
    };
    if !matches!(logical.operator, oxc_ast::ast::LogicalOperator::And) {
        return false;
    }
    // [1] = identifier (the result variable)
    matches!(seq.expressions[1], Expression::Identifier(_))
}

/// `__commonJS`: `(H, $) => () => ($ || H(($ = { exports: {} }).exports, $), $.exports)`
pub(super) fn is_commonjs_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    if arrow.params.items.len() != 2 || !arrow.body.is_expression() {
        return false;
    }
    let Some(inner) = inner_arrow_body_expr(arrow) else {
        return false;
    };
    let Expression::SequenceExpression(seq) = inner else {
        return false;
    };
    if seq.expressions.len() != 2 {
        return false;
    }
    // [0] = `$ || H(...)` — LogicalExpression with || (or LogicalOr)
    let Expression::LogicalExpression(logical) = &seq.expressions[0] else {
        return false;
    };
    if !matches!(logical.operator, oxc_ast::ast::LogicalOperator::Or) {
        return false;
    }
    // [1] = `$.exports` — StaticMemberExpression with property name "exports"
    let Expression::StaticMemberExpression(member) = &seq.expressions[1] else {
        return false;
    };
    member.property.name.as_str() == "exports"
}

/// `__export`: `(target, all) => { for (var name in all) __defProp(target, name, {get: all[name], …}) }`
pub(super) fn is_export_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    if arrow.params.items.len() != 2 || arrow.body.is_expression() {
        return false;
    }
    arrow
        .body
        .as_function_body()
        .and_then(|body| body.statements.first())
        .is_some_and(|stmt| matches!(stmt, oxc_ast::ast::Statement::ForInStatement(_)))
}

/// `__toESM`: `(mod, isNodeMode, target) => …`. 2-3 params, block body, contains __getProtoOf.
pub(super) fn is_toesm_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    let n = arrow.params.items.len();
    if !(2..=3).contains(&n) || arrow.body.is_expression() {
        return false;
    }
    // Heuristic: body contains a reference to a getPrototypeOf-aliased function call,
    // OR contains an `__esModule` string check.
    body_contains_esmodule_check(
        &arrow
            .body
            .as_function_body()
            .expect("block body checked above")
            .statements,
    )
}

/// `__toCommonJS`: `from => { … __esModule: true … }`, 1 param.
pub(super) fn is_tocommonjs_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    if arrow.params.items.len() != 1 || arrow.body.is_expression() {
        return false;
    }
    body_contains_esmodule_string(
        &arrow
            .body
            .as_function_body()
            .expect("block body checked above")
            .statements,
    )
}

/// `__name` arrow form: `(target, name) => { Object.defineProperty(target, "name", …); return target; }`
pub(super) fn is_name_helper_arrow(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    if arrow.params.items.len() != 2 || arrow.body.is_expression() {
        return false;
    }
    body_calls_define_property_with_name(
        &arrow
            .body
            .as_function_body()
            .expect("block body checked above")
            .statements,
    )
}

/// Unwraps the "single `ExpressionStatement` returning an arrow whose body is a single
/// `ExpressionStatement` returning a paren-wrapped sequence" structure used by both
/// `__esm` and `__commonJS`. Returns the inner-most expression after paren-unwrap.
fn inner_arrow_body_expr<'a, 'b>(arrow: &'a oxc_ast::ast::ArrowFunctionExpression<'b>) -> Option<&'a Expression<'b>> {
    let Expression::ArrowFunctionExpression(inner_arrow) = arrow.body.as_expression()? else {
        return None;
    };
    if !inner_arrow.params.items.is_empty() {
        return None;
    }
    let inner = inner_arrow.body.as_expression()?;
    Some(match inner {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    })
}
