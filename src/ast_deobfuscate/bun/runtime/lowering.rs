//! esbuild's syntax-lowering helpers: class fields and private members,
//! object spread/rest, `async` functions, tagged templates, and the `require`
//! / glob-import shims (`internal/runtime/runtime.go`). Each is recognized by
//! its parameter shape plus a call or message only that helper makes.

use oxc_ast::ast::{
    Argument, ArrowFunctionExpression, BinaryOperator, CallExpression, Expression, Statement, VariableDeclarator,
};

use super::ast_utils::{
    arrow_contains, arrow_contains_string, block_body, expression_body, expression_contains, is_ident, is_string,
    param_names,
};
use super::descriptor::{argument_is_ident, call_callee_is_helper, call_callee_is_helper_or_object_property};
use super::string_search::stmt_returns_ident;
use super::types::{BunHelper, BunHelperMap};

/// The error messages of the private-member helpers, which pass them to
/// `__typeError` / `__accessCheck` (or throw them, before esbuild 0.21).
const PRIVATE_MEMBER_MESSAGES: [(&str, BunHelper); 6] = [
    ("read from private field", BunHelper::PrivateGet),
    ("write to private field", BunHelper::PrivateSet),
    ("access private method", BunHelper::PrivateMethod),
    (
        "Cannot add the same private member more than once",
        BunHelper::PrivateAdd,
    ),
    ("Cannot use the \"in\" operator on this value", BunHelper::PrivateIn),
    ("Cannot access \"", BunHelper::EarlyAccess),
];

/// The lowering helper `arrow` implements, if any. `helpers` holds the
/// helpers declared before it, which esbuild always emits first.
pub(super) fn lowering_helper(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> Option<BunHelper> {
    let params = param_names(arrow)?;
    let calls = |helper: BunHelper, args: &dyn Fn(&CallExpression<'_>) -> bool| {
        arrow_contains(
            arrow,
            |expr| matches!(expr, Expression::CallExpression(call) if call_callee_is_helper(call, helpers, helper) && args(call)),
        )
    };
    let helper = match params.as_slice() {
        [msg] if is_type_error_thrower(arrow, msg) => BunHelper::TypeError,
        [map] if arrow_contains_string(arrow, "Module not found in bundle: ") && is_glob(arrow, map) => BunHelper::Glob,
        [a, b] if is_spread_props(arrow, helpers, a, b) => BunHelper::SpreadProps,
        [a, _]
            if block_body(arrow)
                .is_some_and(|body| body.statements.last().is_some_and(|s| stmt_returns_ident(s, a)))
                && calls(BunHelper::DefNormalProp, &|call| first_arg_is(call, a)) =>
        {
            BunHelper::SpreadValues
        }
        [_, exclude]
            if block_body(arrow).is_some()
                && arrow_contains(arrow, |expr| is_method_call_on(expr, exclude, "indexOf")) =>
        {
            BunHelper::ObjRest
        }
        [name, _]
            if arrow_contains_string(arrow, "Symbol.") && arrow_contains(arrow, |expr| is_symbol_read(expr, name)) =>
        {
            BunHelper::KnownSymbol
        }
        [cooked, _]
            if calls(BunHelper::DefProp, &|call| {
                first_arg_is(call, cooked)
                    && call
                        .arguments
                        .get(1)
                        .and_then(Argument::as_expression)
                        .is_some_and(|key| is_string(key, "raw"))
            }) =>
        {
            BunHelper::Template
        }
        [obj, key, _] if is_def_normal_prop(arrow, helpers, obj, key) => BunHelper::DefNormalProp,
        [obj, _, value]
            if calls(BunHelper::DefNormalProp, &|call| {
                first_arg_is(call, obj) && call.arguments.get(2).is_some_and(|arg| argument_is_ident(arg, value))
            }) =>
        {
            BunHelper::PublicField
        }
        [this, args, generator] if is_async(arrow, this, args, generator) => BunHelper::Async,
        [obj, member, _]
            if arrow_contains_string(arrow, "Cannot ")
                && arrow_contains(arrow, |expr| is_member_has(expr, member, obj)) =>
        {
            BunHelper::AccessCheck
        }
        _ => PRIVATE_MEMBER_MESSAGES
            .iter()
            .find(|(message, _)| arrow_contains_string(arrow, message))
            .map(|(_, helper)| *helper)?,
    };
    Some(helper)
}

/// `__require`: `/* @__PURE__ */ (x => typeof require !== "undefined" ? require : …)(function (x) { …
/// throw Error('Dynamic require of "' + x + '" is not supported') })`.
pub(super) fn is_require_shim(decl: &VariableDeclarator<'_>) -> bool {
    let Some(Expression::CallExpression(call)) = decl.init.as_ref().map(Expression::without_parentheses) else {
        return false;
    };
    matches!(
        call.callee.without_parentheses(),
        Expression::ArrowFunctionExpression(_)
    ) && call.arguments.iter().any(|arg| {
        arg.as_expression()
            .is_some_and(|fallback| expression_contains(fallback, |expr| is_string(expr, "Dynamic require of \"")))
    })
}

/// `__typeError`: `msg => { throw TypeError(msg) }`.
fn is_type_error_thrower(arrow: &ArrowFunctionExpression<'_>, msg: &str) -> bool {
    let Some([Statement::ThrowStatement(throw)]) = block_body(arrow).map(|body| body.statements.as_slice()) else {
        return false;
    };
    let (callee, arguments) = match &throw.argument {
        Expression::CallExpression(call) => (&call.callee, &call.arguments),
        Expression::NewExpression(new) => (&new.callee, &new.arguments),
        _ => return false,
    };
    is_ident(callee, "TypeError") && matches!(arguments.as_slice(), [arg] if argument_is_ident(arg, msg))
}

/// `__glob`: `map => path => { var fn = map[path]; if (fn) return fn(); throw … }`.
fn is_glob(arrow: &ArrowFunctionExpression<'_>, map: &str) -> bool {
    matches!(expression_body(arrow), Some(Expression::ArrowFunctionExpression(inner)) if inner.params.items.len() == 1)
        && arrow_contains(
            arrow,
            |expr| matches!(expr, Expression::ComputedMemberExpression(member) if is_ident(&member.object, map)),
        )
}

/// `__spreadProps`: `(a, b) => __defProps(a, __getOwnPropDescs(b))`.
fn is_spread_props(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap, a: &str, b: &str) -> bool {
    let Some(Expression::CallExpression(call)) = expression_body(arrow) else {
        return false;
    };
    call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProps, "defineProperties")
        && matches!(call.arguments.as_slice(), [target, Argument::CallExpression(descs)]
            if argument_is_ident(target, a)
                && call_callee_is_helper_or_object_property(descs, helpers, BunHelper::GetOwnPropDescs, "getOwnPropertyDescriptors")
                && first_arg_is(descs, b))
}

/// `__defNormalProp`: `(obj, key, value) => key in obj ? __defProp(obj, key, {…}) : obj[key] = value`.
fn is_def_normal_prop(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap, obj: &str, key: &str) -> bool {
    let Some(Expression::ConditionalExpression(cond)) = expression_body(arrow) else {
        return false;
    };
    matches!(cond.test.without_parentheses(), Expression::BinaryExpression(test)
        if test.operator == BinaryOperator::In && is_ident(&test.left, key) && is_ident(&test.right, obj))
        && matches!(cond.consequent.without_parentheses(), Expression::CallExpression(call)
            if call_callee_is_helper_or_object_property(call, helpers, BunHelper::DefProp, "defineProperty")
                && first_arg_is(call, obj))
}

/// `__async`: `(__this, __arguments, generator) => new Promise(… generator.apply(__this, __arguments) …)`.
fn is_async(arrow: &ArrowFunctionExpression<'_>, this: &str, args: &str, generator: &str) -> bool {
    arrow_contains(
        arrow,
        |expr| matches!(expr, Expression::NewExpression(new) if is_ident(&new.callee, "Promise")),
    ) && arrow_contains(arrow, |expr| {
        is_method_call_on(expr, generator, "apply")
            && matches!(expr, Expression::CallExpression(call) if matches!(call.arguments.as_slice(),
                    [t, a] if argument_is_ident(t, this) && argument_is_ident(a, args)))
    })
}

/// `member.has(obj)`.
fn is_member_has(expr: &Expression<'_>, member: &str, obj: &str) -> bool {
    is_method_call_on(expr, member, "has")
        && matches!(expr, Expression::CallExpression(call) if matches!(call.arguments.as_slice(), [arg] if argument_is_ident(arg, obj)))
}

/// `Symbol[name]`.
fn is_symbol_read(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::ComputedMemberExpression(member)
        if is_ident(&member.object, "Symbol") && is_ident(&member.expression, name))
}

/// `object.method(…)`.
fn is_method_call_on(expr: &Expression<'_>, object: &str, method: &str) -> bool {
    matches!(expr, Expression::CallExpression(call) if matches!(&call.callee, Expression::StaticMemberExpression(callee)
        if is_ident(&callee.object, object) && callee.property.name == method))
}

fn first_arg_is(call: &CallExpression<'_>, name: &str) -> bool {
    call.arguments.first().is_some_and(|arg| argument_is_ident(arg, name))
}
