//! esbuild's syntax-lowering helpers: class fields and private members,
//! `super` in lowered methods, object spread/rest, `async` functions and
//! generators, `for await`, `using`, TypeScript and JavaScript decorators,
//! tagged templates, the binary loader, and the `require` / glob-import shims
//! (`internal/runtime/runtime.go`). Each is recognized by its parameter shape
//! plus a call or message only that helper makes.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionExpression, AssignmentTarget, BinaryOperator, CallExpression,
    Expression, Statement, UnaryOperator, VariableDeclarator,
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
    let has = |value: &str| arrow_contains_string(arrow, value);
    let calls_with = |helper: BunHelper, names: &[&str]| calls(helper, &|call| args_are(call, names));
    let helper = match params.as_slice() {
        [msg] if is_type_error_thrower(arrow, msg) => BunHelper::TypeError,
        [_] if has("Function expected") => BunHelper::ExpectFn,
        [_] if has("metadata") && calls_create(arrow, helpers) => BunHelper::DecoratorStart,
        [_] if has("asyncIterator") && has("throw") => BunHelper::YieldStar,
        [key] if is_rest_key(arrow, key) => BunHelper::RestKey,
        [_] if has("base64") && arrow_contains(arrow, |expr| is_new_of(expr, "Uint8Array")) => BunHelper::ToBinaryNode,
        [index, decorator] if is_decorate_param(arrow, index, decorator) => BunHelper::DecorateParam,
        [_, _] if has("metadata") && calls(BunHelper::DefNormalProp, &|_| true) => BunHelper::DecoratorMetadata,
        [promise, is_yield_star]
            if assigns_this_index(arrow, 0.0, promise) && assigns_this_index(arrow, 1.0, is_yield_star) =>
        {
            BunHelper::Await
        }
        [this, args, generator] if has("asyncIterator") && is_async(arrow, this, args, generator) => {
            BunHelper::AsyncGenerator
        }
        [_, _, _] if has("asyncIterator") && has("iterator") => BunHelper::ForAwait,
        [_, _, _] if has("Object not disposable") => BunHelper::Using,
        [_, _, _] if has("An error was suppressed during disposal") => BunHelper::CallDispose,
        [cls, obj, key] if is_super_get(arrow, helpers, cls, obj, key) => BunHelper::SuperGet,
        [cls, _, _, val] if is_super_set(arrow, helpers, cls, val) => BunHelper::SuperSet,
        [cls, obj, key] if calls_with(BunHelper::SuperGet, &[cls, obj, key]) => BunHelper::SuperWrapper,
        [obj, member, _, getter] if calls_with(BunHelper::PrivateGet, &[obj, member, getter]) => {
            BunHelper::PrivateWrapper
        }
        [decorators, target, key, _] if is_decorate_class(arrow, helpers, decorators, target, key) => {
            BunHelper::DecorateClass
        }
        [_, _, _, _, _] if has("Already initialized") => BunHelper::DecoratorContext,
        [_, flags, _, _] if arrow_contains(arrow, |expr| is_shift_right_one(expr, flags)) => BunHelper::RunInitializers,
        [_, _, _, _, _, _] if has("Object expected") => BunHelper::DecorateElement,
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

/// `__toBinary`: `(() => { var table = new Uint8Array(128); …; return base64 => { … base64.charCodeAt(i++) … } })()`.
pub(super) fn is_to_binary(expr: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return false;
    };
    call.arguments.is_empty()
        && matches!(call.callee.without_parentheses(), Expression::ArrowFunctionExpression(arrow) if arrow.params.is_empty())
        && expression_contains(expr, |expr| is_new_of(expr, "Uint8Array"))
        && expression_contains(expr, |expr| {
            matches!(expr, Expression::CallExpression(call)
            if matches!(&call.callee, Expression::StaticMemberExpression(member) if member.property.name == "charCodeAt"))
        })
}

/// `__decoratorStrings`: `["class", "method", "getter", "setter", "accessor", "field", "value", "get", "set"]`.
pub(super) fn is_decorator_strings(expr: &Expression<'_>) -> bool {
    const STRINGS: [&str; 9] = [
        "class", "method", "getter", "setter", "accessor", "field", "value", "get", "set",
    ];
    matches!(expr.without_parentheses(), Expression::ArrayExpression(array)
        if array.elements.len() == STRINGS.len()
            && array.elements.iter().zip(STRINGS).all(|(element, value)|
                matches!(element, ArrayExpressionElement::StringLiteral(lit) if lit.value == value)))
}

/// `__restKey`: `key => typeof key === "symbol" ? key : key + ""`.
fn is_rest_key(arrow: &ArrowFunctionExpression<'_>, key: &str) -> bool {
    let Some(Expression::ConditionalExpression(cond)) = expression_body(arrow) else {
        return false;
    };
    matches!(cond.test.without_parentheses(), Expression::BinaryExpression(test)
        if matches!(test.operator, BinaryOperator::Equality | BinaryOperator::StrictEquality)
            && matches!(&test.left, Expression::UnaryExpression(unary)
                if unary.operator == UnaryOperator::Typeof && is_ident(&unary.argument, key))
            && is_string(&test.right, "symbol"))
        && is_ident(&cond.consequent, key)
        && matches!(cond.alternate.without_parentheses(), Expression::BinaryExpression(concat)
            if concat.operator == BinaryOperator::Addition && is_ident(&concat.left, key) && is_string(&concat.right, ""))
}

/// `__decorateParam`: `(index, decorator) => (target, key) => decorator(target, key, index)`.
fn is_decorate_param(arrow: &ArrowFunctionExpression<'_>, index: &str, decorator: &str) -> bool {
    let Some(Expression::ArrowFunctionExpression(inner)) = expression_body(arrow) else {
        return false;
    };
    let Some(&[target, key]) = param_names(inner).as_deref() else {
        return false;
    };
    matches!(expression_body(inner), Some(Expression::CallExpression(call))
        if is_ident(&call.callee, decorator) && args_are(call, &[target, key, index]))
}

/// `__decorateClass`: `(decorators, target, key, kind) => { var result = … __getOwnPropDesc(target, key) …;
/// for (var i = decorators.length - 1, decorator; i >= 0; i--) … }`.
fn is_decorate_class(
    arrow: &ArrowFunctionExpression<'_>,
    helpers: &BunHelperMap,
    decorators: &str,
    target: &str,
    key: &str,
) -> bool {
    arrow_contains(arrow, |expr| {
        matches!(expr, Expression::StaticMemberExpression(member)
            if is_ident(&member.object, decorators) && member.property.name == "length")
    }) && arrow_contains(arrow, |expr| {
        matches!(expr, Expression::CallExpression(call)
            if call_callee_is_helper_or_object_property(call, helpers, BunHelper::GetOwnPropDesc, "getOwnPropertyDescriptor")
                && args_are(call, &[target, key]))
    })
}

/// `__superGet`: `(cls, obj, key) => __reflectGet(__getProtoOf(cls), key, obj)`.
fn is_super_get(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap, cls: &str, obj: &str, key: &str) -> bool {
    matches!(expression_body(arrow), Some(Expression::CallExpression(call))
        if is_reflect_call(call, helpers, BunHelper::ReflectGet, "get")
            && matches!(call.arguments.as_slice(), [Argument::CallExpression(proto), k, o]
                if is_get_proto_of(proto, helpers, cls) && argument_is_ident(k, key) && argument_is_ident(o, obj)))
}

/// `__superSet`: `(cls, obj, key, val) => (__reflectSet(__getProtoOf(cls), key, val, obj), val)`.
fn is_super_set(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap, cls: &str, val: &str) -> bool {
    let Some(Expression::SequenceExpression(seq)) = expression_body(arrow) else {
        return false;
    };
    matches!(seq.expressions.as_slice(), [Expression::CallExpression(call), result]
        if is_ident(result, val)
            && is_reflect_call(call, helpers, BunHelper::ReflectSet, "set")
            && matches!(call.arguments.first(), Some(Argument::CallExpression(proto)) if is_get_proto_of(proto, helpers, cls)))
}

/// A call of the `Reflect.<method>` alias `helper`, or of `Reflect.<method>` itself.
fn is_reflect_call(call: &CallExpression<'_>, helpers: &BunHelperMap, helper: BunHelper, method: &str) -> bool {
    call_callee_is_helper(call, helpers, helper)
        || matches!(&call.callee, Expression::StaticMemberExpression(member)
            if is_ident(&member.object, "Reflect") && member.property.name == method)
}

/// `__getProtoOf(cls)`.
fn is_get_proto_of(call: &CallExpression<'_>, helpers: &BunHelperMap, cls: &str) -> bool {
    call_callee_is_helper_or_object_property(call, helpers, BunHelper::GetProtoOf, "getPrototypeOf")
        && args_are(call, &[cls])
}

/// `__decoratorStart` creates the metadata object with `__create(…)`.
fn calls_create(arrow: &ArrowFunctionExpression<'_>, helpers: &BunHelperMap) -> bool {
    arrow_contains(arrow, |expr| {
        matches!(expr, Expression::CallExpression(call)
            if call_callee_is_helper_or_object_property(call, helpers, BunHelper::Create, "create"))
    })
}

/// `this[index] = value`, as `__await` stores its promise and `yield*` flag.
fn assigns_this_index(arrow: &ArrowFunctionExpression<'_>, index: f64, value: &str) -> bool {
    arrow_contains(arrow, |expr| {
        matches!(expr, Expression::AssignmentExpression(assign)
            if is_ident(&assign.right, value)
                && matches!(&assign.left, AssignmentTarget::ComputedMemberExpression(member)
                    if matches!(member.object, Expression::ThisExpression(_))
                        && matches!(&member.expression, Expression::NumericLiteral(n) if n.value == index)))
    })
}

/// `flags >> 1`, the initializer slot `__runInitializers` reads.
fn is_shift_right_one(expr: &Expression<'_>, flags: &str) -> bool {
    matches!(expr, Expression::BinaryExpression(shift)
        if shift.operator == BinaryOperator::ShiftRight
            && is_ident(&shift.left, flags)
            && matches!(&shift.right, Expression::NumericLiteral(n) if n.value == 1.0))
}

/// `new Callee(…)`.
fn is_new_of(expr: &Expression<'_>, callee: &str) -> bool {
    matches!(expr, Expression::NewExpression(new) if is_ident(&new.callee, callee))
}

/// Whether the call's arguments are exactly the identifiers `names`.
fn args_are(call: &CallExpression<'_>, names: &[&str]) -> bool {
    call.arguments.len() == names.len()
        && call
            .arguments
            .iter()
            .zip(names)
            .all(|(arg, name)| argument_is_ident(arg, name))
}
