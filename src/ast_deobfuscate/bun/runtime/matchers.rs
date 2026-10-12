use oxc_ast::ast::{
    Argument, ArrowFunctionExpression, Expression, ForStatementLeft, LogicalOperator, NewExpression,
    ObjectPropertyKind, Statement,
};
use oxc_ast_visit::{Visit, walk};

use super::ast_utils::{arrow_contains, expression_body, is_ident, is_string, param_names};
use super::descriptor::{call_callee_is_helper, call_callee_is_helper_or_object_property};
use super::string_search::{
    body_calls_define_property_with_name, body_contains_esmodule_check, body_contains_esmodule_string,
};
use super::types::{BunHelper, BunHelperMap};

/// `__esm`: the returned function evaluates the module once, `fn && (res = fn(fn = 0)), res`.
pub(super) fn is_esm_helper(arrow: &ArrowFunctionExpression<'_>) -> bool {
    let Some((params, value)) = lazy_wrapper(arrow) else {
        return false;
    };
    let Expression::SequenceExpression(seq) = value else {
        return false;
    };
    let [Expression::LogicalExpression(logical), result] = seq.expressions.as_slice() else {
        return false;
    };
    logical.operator == LogicalOperator::And
        && is_ident(&logical.left, params[0])
        && matches!(logical.right.without_parentheses(), Expression::AssignmentExpression(assign)
            if assign.left.get_identifier_name().is_some_and(|name| name == params[1]))
        && is_ident(result, params[1])
}

/// `__commonJS`: the returned function runs the module once and returns its
/// exports, `mod || cb((mod = { exports: {} }).exports, mod), mod.exports`.
pub(super) fn is_commonjs_helper(arrow: &ArrowFunctionExpression<'_>) -> bool {
    let Some((params, value)) = lazy_wrapper(arrow) else {
        return false;
    };
    let Expression::SequenceExpression(seq) = value else {
        return false;
    };
    let [
        Expression::LogicalExpression(logical),
        Expression::StaticMemberExpression(exports),
    ] = seq.expressions.as_slice()
    else {
        return false;
    };
    logical.operator == LogicalOperator::Or
        && is_ident(&logical.left, params[1])
        && is_ident(&exports.object, params[1])
        && exports.property.name == "exports"
}

/// The parameters of a lazy module wrapper helper (`__esm` / `__commonJS`)
/// and the value its returned function returns. The returned function is an
/// arrow when minified and `function __init()` / `function __require()`
/// otherwise; since esbuild 0.28 its body is wrapped in `try` (and `__esm`
/// takes an `err` parameter that rethrows the first failure):
///
/// ```js
/// (fn, res) => () => (fn && (res = fn(fn = 0)), res)
/// (fn, res) => function __init() { return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res }
/// (fn, res, err) => () => { if (err) throw err[0]; try { return fn && (res = fn(fn = 0)), res } catch (e) { throw err = [e], e } }
/// (cb, mod) => () => { try { return mod || cb((mod = { exports: {} }).exports, mod), mod.exports } catch (e) { throw mod = 0, e } }
/// ```
fn lazy_wrapper<'a, 'b>(arrow: &'a ArrowFunctionExpression<'b>) -> Option<(Vec<&'a str>, &'a Expression<'b>)> {
    let params = param_names(arrow)?;
    if !(2..=3).contains(&params.len()) {
        return None;
    }
    let statements = match expression_body(arrow)? {
        Expression::ArrowFunctionExpression(inner) if inner.params.is_empty() => match expression_body(inner) {
            Some(value) if params.len() == 2 => return Some((params, value)),
            Some(_) => return None,
            None => &inner.body.as_function_body()?.statements,
        },
        Expression::FunctionExpression(inner) if inner.params.is_empty() => &inner.body.as_ref()?.statements,
        _ => return None,
    };
    let statements = match (statements.as_slice(), params.get(2)) {
        ([Statement::IfStatement(rethrow), rest @ ..], Some(err))
            if rethrow.alternate.is_none()
                && is_ident(&rethrow.test, err)
                && matches!(&rethrow.consequent, Statement::ThrowStatement(_)) =>
        {
            rest
        }
        (statements, None) => statements,
        _ => return None,
    };
    let value = match statements {
        [Statement::ReturnStatement(ret)] => ret.argument.as_ref()?,
        [Statement::TryStatement(try_stmt)] if try_stmt.handler.is_some() && try_stmt.finalizer.is_none() => {
            let [Statement::ReturnStatement(ret)] = try_stmt.block.body.as_slice() else {
                return None;
            };
            ret.argument.as_ref()?
        }
        _ => return None,
    };
    Some((params, value.without_parentheses()))
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

/// `__toESM`: `(mod, isNodeMode, target) => …` that copies `mod` onto a fresh
/// object (`__create(__getProtoOf(mod))`) unless it is `__esModule`; Bun's has
/// a block body, esbuild's is one sequence expression.
pub(super) fn is_toesm_helper(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    if !(2..=3).contains(&arrow.params.items.len()) {
        return false;
    }
    if let Some(body) = arrow.body.as_function_body() {
        return body_contains_esmodule_check(&body.statements);
    }
    arrow_contains(arrow, is_esmodule_read)
        && arrow_contains(arrow, |expr| {
            matches!(expr, Expression::CallExpression(call)
                if call_callee_is_helper_or_object_property(call, helpers, BunHelper::Create, "create"))
        })
}

/// `__toCommonJS`: Bun's `from => { … __esModule: true … }`, or esbuild's
/// `mod => __copyProps(__defProp({}, "__esModule", { value: true }), mod)`.
pub(super) fn is_tocommonjs_helper(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    let Some(&[param]) = param_names(arrow).as_deref() else {
        return false;
    };
    if let Some(Expression::CallExpression(copy)) = expression_body(arrow) {
        return call_callee_is_helper(copy, helpers, BunHelper::CopyProps)
            && matches!(copy.arguments.as_slice(), [Argument::CallExpression(target), Argument::Identifier(from)]
                if from.name == param && target.arguments.get(1).and_then(Argument::as_expression)
                    .is_some_and(|key| is_string(key, "__esModule")));
    }
    arrow
        .body
        .as_function_body()
        .is_some_and(|body| body_contains_esmodule_string(&body.statements))
}

/// `__name`: `(target, value) => __defProp(target, "name", { value, configurable: true })`,
/// or a block body that makes such a call.
pub(super) fn is_name_helper_arrow(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    let Some(&[target, _]) = param_names(arrow).as_deref() else {
        return false;
    };
    match expression_body(arrow) {
        Some(Expression::CallExpression(call)) => {
            call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProp, "defineProperty")
                && matches!(call.arguments.as_slice(), [Argument::Identifier(t), Argument::StringLiteral(key), Argument::ObjectExpression(_)]
                    if t.name == target && key.value == "name")
        }
        Some(_) => false,
        None => arrow
            .body
            .as_function_body()
            .is_some_and(|body| body_calls_define_property_with_name(&body.statements)),
    }
}

fn is_esmodule_read(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::StaticMemberExpression(member) if member.property.name == "__esModule")
}
