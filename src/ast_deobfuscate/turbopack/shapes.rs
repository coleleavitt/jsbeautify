//! Turbopack chunk shapes, shared by the naming pass and
//! [`crate::turbopack_extract`].
//!
//! A browser chunk registers its module factories with the runtime through the
//! chunk loading global:
//!
//! ```js
//! (globalThis.TURBOPACK || (globalThis.TURBOPACK = [])).push([
//!     script,            // chunk path string or `document.currentScript`
//!     12, (e) => {…},    // module id, factory
//!     34, 56, (e, t, r) => {…}, // several ids sharing one scope-hoisted factory
//! ]);
//! ```
//!
//! The runtime's `installCompressedModuleFactories` reads the list after
//! `script` as runs of module ids, each run closed by its factory. A chunk may
//! put its strict factories first, inside `(() => { "use strict"; return [ … ] })()`.
//! The runtime chunk instead registers `[script, { otherChunks, runtimeModuleIds }]`.

use oxc_ast::ast::{
    ArrayExpression, ArrayExpressionElement, ArrowFunctionBody, AssignmentTarget, CallExpression, Expression,
    FormalParameters, FunctionBody, ObjectExpression, ObjectPropertyKind, Statement,
};
use oxc_syntax::operator::LogicalOperator;

/// One run of module ids and the factory they share.
pub struct FactoryGroup<'b, 'a> {
    pub ids: Vec<String>,
    pub factory: &'b Expression<'a>,
    /// The factory sits in the `"use strict"` IIFE of a mixed chunk.
    pub strict_wrapper: bool,
}

pub enum Payload<'b, 'a> {
    Factories(Vec<FactoryGroup<'b, 'a>>),
    /// `{ otherChunks, runtimeModuleIds }` of the runtime chunk.
    Runtime(&'b ObjectExpression<'a>),
}

/// A `TURBOPACK.push([script, …])` chunk registration.
pub struct Registration<'b, 'a> {
    pub script: &'b Expression<'a>,
    pub payload: Payload<'b, 'a>,
}

const GLOBAL_OBJECTS: [&str; 4] = ["globalThis", "self", "window", "global"];

/// `globalThis.TURBOPACK` / `self["TURBOPACK"]`: the chunk loading global,
/// whose name Turbopack configures but always starts with `TURBOPACK`.
fn is_loading_global_member(object: &Expression<'_>, name: Option<&str>) -> bool {
    matches!(object.without_parentheses(), Expression::Identifier(id) if GLOBAL_OBJECTS.contains(&id.name.as_str()))
        && name.is_some_and(|name| name.starts_with("TURBOPACK"))
}

fn is_loading_global(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::StaticMemberExpression(m) => is_loading_global_member(&m.object, Some(m.property.name.as_str())),
        Expression::ComputedMemberExpression(m) => {
            is_loading_global_member(&m.object, m.static_property_name().as_deref())
        }
        _ => false,
    }
}

fn is_loading_global_target(target: &AssignmentTarget<'_>) -> bool {
    match target {
        AssignmentTarget::StaticMemberExpression(m) => {
            is_loading_global_member(&m.object, Some(m.property.name.as_str()))
        }
        AssignmentTarget::ComputedMemberExpression(m) => {
            is_loading_global_member(&m.object, m.static_property_name().as_deref())
        }
        _ => false,
    }
}

/// `G` or `G || (G = [])` for the chunk loading global `G`.
fn is_registry(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::LogicalExpression(or) if or.operator == LogicalOperator::Or => {
            is_loading_global(&or.left)
                && matches!(or.right.without_parentheses(), Expression::AssignmentExpression(init)
                    if is_loading_global_target(&init.left)
                        && matches!(&init.right, Expression::ArrayExpression(a) if a.elements.is_empty()))
        }
        other => is_loading_global(other),
    }
}

/// A module id literal: a number in production, a string in development.
pub fn module_id(expr: &Expression<'_>) -> Option<String> {
    match expr.without_parentheses() {
        Expression::NumericLiteral(n) if n.value >= 0.0 && n.value.fract() == 0.0 => Some(n.value.to_string()),
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// Parameters of a module factory: a plain function or arrow.
pub fn factory_params<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b FormalParameters<'a>> {
    match expr.without_parentheses() {
        Expression::FunctionExpression(func) if !func.r#async && !func.generator => Some(&func.params),
        Expression::ArrowFunctionExpression(arrow) if !arrow.r#async => Some(&arrow.params),
        _ => None,
    }
}

pub fn has_use_strict(body: &FunctionBody<'_>) -> bool {
    body.directives.iter().any(|d| d.directive == "use strict")
}

/// `(() => { "use strict"; return [ … ] })()` (or the `function` form).
fn strict_factory_list<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b ArrayExpression<'a>> {
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    if !call.arguments.is_empty() {
        return None;
    }
    let (params, body) = match call.callee.without_parentheses() {
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) => (&arrow.params, &**body),
            _ => return None,
        },
        Expression::FunctionExpression(func) => (&func.params, func.body.as_deref()?),
        _ => return None,
    };
    if !params.items.is_empty() || params.rest.is_some() || !has_use_strict(body) {
        return None;
    }
    let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
        return None;
    };
    match ret.argument.as_ref()?.without_parentheses() {
        Expression::ArrayExpression(list) => Some(list),
        _ => None,
    }
}

/// Appends the groups of a compressed factory list; `None` unless every
/// element is a module id or a factory and the list ends with a factory.
fn push_groups<'b, 'a>(
    elements: &'b [ArrayExpressionElement<'a>],
    strict_wrapper: bool,
    out: &mut Vec<FactoryGroup<'b, 'a>>,
) -> Option<()> {
    let mut ids = Vec::new();
    for element in elements {
        let expr = element.as_expression()?;
        if let Some(id) = module_id(expr) {
            ids.push(id);
        } else if !ids.is_empty() && factory_params(expr).is_some() {
            out.push(FactoryGroup {
                ids: std::mem::take(&mut ids),
                factory: expr,
                strict_wrapper,
            });
        } else {
            return None;
        }
    }
    ids.is_empty().then_some(())
}

fn is_runtime_params(obj: &ObjectExpression<'_>) -> bool {
    obj.properties.iter().any(|prop| {
        matches!(prop, ObjectPropertyKind::ObjectProperty(p)
            if p.key.is_specific_static_name("otherChunks") || p.key.is_specific_static_name("runtimeModuleIds"))
    })
}

/// The chunk registration `call` makes, if it is one.
pub fn registration<'b, 'a>(call: &'b CallExpression<'a>) -> Option<Registration<'b, 'a>> {
    let Expression::StaticMemberExpression(push) = call.callee.without_parentheses() else {
        return None;
    };
    if push.property.name != "push" || !is_registry(&push.object) {
        return None;
    }
    let [arg] = call.arguments.as_slice() else {
        return None;
    };
    let Expression::ArrayExpression(list) = arg.as_expression()?.without_parentheses() else {
        return None;
    };
    let (script, rest) = list.elements.split_first()?;
    let script = script.as_expression()?;

    if let [ArrayExpressionElement::ObjectExpression(params)] = rest
        && is_runtime_params(params)
    {
        return Some(Registration {
            script,
            payload: Payload::Runtime(params),
        });
    }

    let mut groups = Vec::new();
    let mut rest = rest;
    if let Some((first, tail)) = rest.split_first()
        && let Some(strict) = first.as_expression().and_then(strict_factory_list)
    {
        push_groups(&strict.elements, true, &mut groups)?;
        rest = tail;
    }
    push_groups(rest, false, &mut groups)?;
    (!groups.is_empty()).then_some(Registration {
        script,
        payload: Payload::Factories(groups),
    })
}

/// One binding of an `esmExport` list.
pub enum EsmBinding<'b, 'a> {
    /// `name, 0, value`
    Value(&'b Expression<'a>),
    /// `name, getter` or `name, getter, setter`
    Getter(&'b Expression<'a>),
}

/// The bindings of `__turbopack_context__.s([…])`, read like the runtime's
/// `esm()`: after a name, a numeric tag `0` introduces a value; anything else
/// is a getter, followed by an optional setter function. Stops at the first
/// element it cannot read.
pub fn esm_bindings<'b, 'a>(list: &'b ArrayExpression<'a>) -> Vec<(&'b str, EsmBinding<'b, 'a>)> {
    let mut out = Vec::new();
    let mut elements = list.elements.iter().peekable();
    while let Some(ArrayExpressionElement::StringLiteral(name)) = elements.next() {
        let Some(binding) = elements.next().and_then(ArrayExpressionElement::as_expression) else {
            break;
        };
        if matches!(binding, Expression::NumericLiteral(tag) if tag.value == 0.0) {
            let Some(value) = elements.next().and_then(ArrayExpressionElement::as_expression) else {
                break;
            };
            out.push((name.value.as_str(), EsmBinding::Value(value)));
            continue;
        }
        out.push((name.value.as_str(), EsmBinding::Getter(binding)));
        match elements.peek() {
            None | Some(ArrayExpressionElement::StringLiteral(_)) => {}
            Some(next) if next.as_expression().and_then(factory_params).is_some() => {
                elements.next();
            }
            Some(_) => break,
        }
    }
    out
}
