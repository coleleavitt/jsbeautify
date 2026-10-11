use oxc_ast::ast::{Argument, Expression, ForStatementLeft, NewExpression, ObjectPropertyKind, Statement};
use oxc_ast_visit::{Visit, walk};

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

/// `__export`: `(target, all) => { for (var name in all) __defProp(target, name, { get: all[name], … }) }`
pub(super) fn is_export_helper(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    let [target, all] = arrow.params.items.as_slice() else {
        return false;
    };
    let (Some(target), Some(all)) = (
        target.pattern.get_binding_identifier(),
        all.pattern.get_binding_identifier(),
    ) else {
        return false;
    };
    let Some([Statement::ForInStatement(for_in)]) =
        arrow.body.as_function_body().map(|body| body.statements.as_slice())
    else {
        return false;
    };
    let ForStatementLeft::VariableDeclaration(left) = &for_in.left else {
        return false;
    };
    let Some(name) = left
        .declarations
        .first()
        .and_then(|decl| decl.id.get_binding_identifier())
    else {
        return false;
    };
    let is_ident = |expr: &Expression<'_>, ident: &str| matches!(expr, Expression::Identifier(id) if id.name == ident);
    if !is_ident(&for_in.right, &all.name) {
        return false;
    }
    let Statement::ExpressionStatement(stmt) = &for_in.body else {
        return false;
    };
    let Expression::CallExpression(call) = &stmt.expression else {
        return false;
    };
    let [
        Argument::Identifier(arg_target),
        Argument::Identifier(arg_name),
        Argument::ObjectExpression(descriptor),
    ] = call.arguments.as_slice()
    else {
        return false;
    };
    arg_target.name == target.name
        && arg_name.name == name.name
        && descriptor.properties.iter().any(|property| {
            matches!(property, ObjectPropertyKind::ObjectProperty(prop)
                if prop.key.static_name().is_some_and(|key| key == "get")
                    && matches!(&prop.value, Expression::ComputedMemberExpression(member)
                        if is_ident(&member.object, &all.name) && is_ident(&member.expression, &name.name)))
        })
}

/// The property keys of the descriptor an `__export` helper passes to
/// `__defProp`: `{ get, enumerable }` from esbuild, plus `configurable` and
/// `set` from Bun.
pub(super) fn export_descriptor_keys(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> Vec<String> {
    let Some(Statement::ForInStatement(for_in)) =
        arrow.body.as_function_body().and_then(|body| body.statements.first())
    else {
        return Vec::new();
    };
    let Statement::ExpressionStatement(stmt) = &for_in.body else {
        return Vec::new();
    };
    let Expression::CallExpression(call) = &stmt.expression else {
        return Vec::new();
    };
    let Some(Argument::ObjectExpression(descriptor)) = call.arguments.get(2) else {
        return Vec::new();
    };
    descriptor
        .properties
        .iter()
        .filter_map(|property| match property {
            ObjectPropertyKind::ObjectProperty(prop) => prop.key.static_name().map(String::from),
            ObjectPropertyKind::SpreadProperty(_) => None,
        })
        .collect()
}

/// Whether the helper body constructs a `WeakMap`, as Bun's cached
/// `__toESM` / `__toCommonJS` do.
pub(super) fn constructs_weak_map(arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> bool {
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
            self.0 |= matches!(&new.callee, Expression::Identifier(id) if id.name == "WeakMap");
            walk::walk_new_expression(self, new);
        }
    }
    let mut finder = Finder(false);
    finder.visit_arrow_function_expression(arrow);
    finder.0
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
