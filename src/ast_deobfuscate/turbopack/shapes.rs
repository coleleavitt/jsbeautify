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
//! put its strict factories first, inside `(() => { "use strict"; return [ … ] })()`,
//! or wrap the whole chunk in `(() => { "use strict"; … })()` when every factory
//! is strict (`turbopack-ecmascript/src/chunk/factory_group.rs`).
//! The runtime chunk instead registers `[script, { otherChunks, runtimeModuleIds }]`.
//!
//! A Node.js chunk (`turbopack-nodejs/src/ecmascript/node/content.rs`) exports the
//! same list without `script`, and the runtime `require`s it:
//!
//! ```js
//! module.exports = [12, (__turbopack_context__, module, exports) => {…}];
//! ```
//!
//! Module ids are numbers in production and strings such as
//! `"[project]/src/foo.ts [app-client] (ecmascript)"` in development.

use std::borrow::Cow;

use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionBody, AssignmentExpression, AssignmentTarget,
    CallExpression, Expression, FormalParameters, FunctionBody, ObjectExpression, ObjectPropertyKind, Program,
    Statement,
};
use oxc_syntax::operator::{AssignmentOperator, LogicalOperator};

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

/// A `TURBOPACK.push([script, …])` chunk registration, or the factory list a
/// Node.js chunk assigns to `module.exports` (no `script`).
pub struct Registration<'b, 'a> {
    pub script: Option<&'b Expression<'a>>,
    pub payload: Payload<'b, 'a>,
}

const GLOBAL_OBJECTS: [&str; 4] = ["globalThis", "self", "window", "global"];

/// The name of `globalThis.G` / `self["G"]`, a global the chunk loading
/// global could be.
fn global_member_name<'b>(object: &Expression<'_>, name: Option<Cow<'b, str>>) -> Option<Cow<'b, str>> {
    matches!(object.without_parentheses(), Expression::Identifier(id) if GLOBAL_OBJECTS.contains(&id.name.as_str()))
        .then_some(name)
        .flatten()
}

fn global_name<'b>(expr: &'b Expression<'_>) -> Option<Cow<'b, str>> {
    match expr.without_parentheses() {
        Expression::StaticMemberExpression(m) => {
            global_member_name(&m.object, Some(Cow::Borrowed(m.property.name.as_str())))
        }
        Expression::ComputedMemberExpression(m) => {
            global_member_name(&m.object, m.static_property_name().map(Cow::from))
        }
        _ => None,
    }
}

fn global_target_name<'b>(target: &'b AssignmentTarget<'_>) -> Option<Cow<'b, str>> {
    match target {
        AssignmentTarget::StaticMemberExpression(m) => {
            global_member_name(&m.object, Some(Cow::Borrowed(m.property.name.as_str())))
        }
        AssignmentTarget::ComputedMemberExpression(m) => {
            global_member_name(&m.object, m.static_property_name().map(Cow::from))
        }
        _ => None,
    }
}

/// The name of the chunk loading global `G` in `G || (G = [])` or a bare `G`
/// named `TURBOPACK…` (Next.js always uses a `TURBOPACK` prefix; Turbopack
/// itself lets the name be configured). `G` ends with `suffix`
/// (`_CHUNK_LISTS` names the chunk list global).
fn registry_name<'b>(expr: &'b Expression<'_>, suffix: &str) -> Option<Cow<'b, str>> {
    let name = match expr.without_parentheses() {
        Expression::LogicalExpression(or) if or.operator == LogicalOperator::Or => {
            let Expression::AssignmentExpression(init) = or.right.without_parentheses() else {
                return None;
            };
            if !matches!(&init.right, Expression::ArrayExpression(a) if a.elements.is_empty()) {
                return None;
            }
            global_name(&or.left).filter(|name| global_target_name(&init.left).as_ref() == Some(name))?
        }
        other => global_name(other).filter(|name| name.starts_with("TURBOPACK"))?,
    };
    name.ends_with(suffix).then_some(name)
}

/// The chunk path string, or the `document.currentScript` lookup.
fn is_script_element(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::StringLiteral(_) | Expression::ConditionalExpression(_) => true,
        Expression::StaticMemberExpression(m) => m.property.name == "currentScript",
        _ => false,
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

/// The body of `(() => { "use strict"; … })()` (or the `function` form).
fn strict_iife_body<'b, 'a>(call: &'b CallExpression<'a>) -> Option<&'b FunctionBody<'a>> {
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
    (params.items.is_empty() && params.rest.is_none() && has_use_strict(body)).then_some(body)
}

/// `(() => { "use strict"; return [ … ] })()` (or the `function` form).
fn strict_factory_list<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b ArrayExpression<'a>> {
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    let body = strict_iife_body(call)?;
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

/// The registry name and single argument of `<registry>.push(arg)`.
fn push_argument<'b, 'a>(call: &'b CallExpression<'a>, suffix: &str) -> Option<(Cow<'b, str>, &'b Expression<'a>)> {
    let Expression::StaticMemberExpression(push) = call.callee.without_parentheses() else {
        return None;
    };
    if push.property.name != "push" {
        return None;
    }
    let registry = registry_name(&push.object, suffix)?;
    let [arg] = call.arguments.as_slice() else {
        return None;
    };
    Some((registry, arg.as_expression()?.without_parentheses()))
}

/// The chunk registration `call` makes, if it is one.
pub fn registration<'b, 'a>(call: &'b CallExpression<'a>) -> Option<Registration<'b, 'a>> {
    let (registry, arg) = push_argument(call, "")?;
    let list = match arg {
        Expression::ArrayExpression(list) => list,
        // Next.js inlines the entry's bootstrap into the page, without a script
        // (`turbopack-browser/src/chunking_context.rs`, shared runtime).
        Expression::ObjectExpression(params) if is_runtime_params(params) => {
            return Some(Registration {
                script: None,
                payload: Payload::Runtime(params),
            });
        }
        _ => return None,
    };
    let (script, rest) = list.elements.split_first()?;
    let script = script.as_expression()?;
    if !registry.starts_with("TURBOPACK") && !is_script_element(script) {
        return None;
    }
    let script = Some(script);

    if let [ArrayExpressionElement::ObjectExpression(params)] = rest
        && is_runtime_params(params)
    {
        return Some(Registration {
            script,
            payload: Payload::Runtime(params),
        });
    }
    Some(Registration {
        script,
        payload: Payload::Factories(factory_groups(rest)?),
    })
}

/// The groups of a compressed factory list, optionally led by the strict
/// factories' IIFE; `None` unless the whole list reads and is not empty.
fn factory_groups<'b, 'a>(elements: &'b [ArrayExpressionElement<'a>]) -> Option<Vec<FactoryGroup<'b, 'a>>> {
    let mut groups = Vec::new();
    let mut rest = elements;
    if let Some((first, tail)) = rest.split_first()
        && let Some(strict) = first.as_expression().and_then(strict_factory_list)
    {
        push_groups(&strict.elements, true, &mut groups)?;
        rest = tail;
    }
    push_groups(rest, false, &mut groups)?;
    (!groups.is_empty()).then_some(groups)
}

/// `module.exports = [id, factory, …]`: the module factories of a Node.js
/// chunk. Factories take at most `(__turbopack_context__, module, exports)`.
pub fn node_chunk<'b, 'a>(assign: &'b AssignmentExpression<'a>) -> Option<Registration<'b, 'a>> {
    let AssignmentTarget::StaticMemberExpression(target) = &assign.left else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign
        || target.property.name != "exports"
        || !matches!(&target.object, Expression::Identifier(id) if id.name == "module")
    {
        return None;
    }
    let Expression::ArrayExpression(list) = assign.right.without_parentheses() else {
        return None;
    };
    let groups = factory_groups(&list.elements)?;
    groups
        .iter()
        .all(|group| factory_params(group.factory).is_some_and(|p| p.items.len() <= 3 && p.rest.is_none()))
        .then_some(Registration {
            script: None,
            payload: Payload::Factories(groups),
        })
}

/// The Node.js chunk among `program`'s top-level statements: a
/// `module.exports = […]` statement, possibly inside the all-strict
/// `(() => { "use strict"; … })()` wrapper.
pub fn program_node_chunk<'b, 'a>(program: &'b Program<'a>) -> Option<Registration<'b, 'a>> {
    program.body.iter().find_map(|statement| {
        let Statement::ExpressionStatement(stmt) = statement else {
            return None;
        };
        match stmt.expression.without_parentheses() {
            Expression::AssignmentExpression(assign) => node_chunk(assign),
            Expression::CallExpression(call) => {
                let [Statement::ExpressionStatement(inner)] = strict_iife_body(call)?.statements.as_slice() else {
                    return None;
                };
                let Expression::AssignmentExpression(assign) = inner.expression.without_parentheses() else {
                    return None;
                };
                node_chunk(assign)
            }
            _ => None,
        }
    })
}

/// A `(globalThis.TURBOPACK_CHUNK_LISTS || (… = [])).push({ script, chunks, source })`
/// chunk list registration (`turbopack-browser/src/ecmascript/list/content.rs`):
/// the chunks a development page loads, which the runtime watches for HMR.
pub struct ChunkList<'b, 'a> {
    pub script: &'b Expression<'a>,
    pub chunks: &'b Expression<'a>,
}

/// The chunk list registration `call` makes, if it is one.
pub fn chunk_list<'b, 'a>(call: &'b CallExpression<'a>) -> Option<ChunkList<'b, 'a>> {
    let Expression::ObjectExpression(params) = push_argument(call, "_CHUNK_LISTS")?.1 else {
        return None;
    };
    let property = |name: &str| {
        params.properties.iter().find_map(|prop| match prop {
            ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name(name) => Some(&p.value),
            _ => None,
        })
    };
    Some(ChunkList {
        script: property("script")?,
        chunks: property("chunks")?,
    })
}

/// `var R = require("…/[turbopack]_runtime.js")("server/app/page.js")`: a
/// Node.js entry (`turbopack-nodejs/src/ecmascript/node/entry/chunk.rs`) loads
/// its chunks with `R.c(path)`, evaluates modules with `R.m(id)` and exports
/// `R.m(id).exports`. Returns the runtime request and the entry's own path.
pub fn node_entry_runtime<'b>(init: &'b Expression<'_>) -> Option<(&'b str, &'b str)> {
    let Expression::CallExpression(outer) = init.without_parentheses() else {
        return None;
    };
    let Expression::CallExpression(require) = outer.callee.without_parentheses() else {
        return None;
    };
    let ([Argument::StringLiteral(runtime)], [Argument::StringLiteral(path)]) =
        (require.arguments.as_slice(), outer.arguments.as_slice())
    else {
        return None;
    };
    (matches!(require.callee.without_parentheses(), Expression::Identifier(id) if id.name == "require")
        && runtime.value.ends_with("_runtime.js"))
    .then(|| (runtime.value.as_str(), path.value.as_str()))
}

/// A readable identifier fragment for a module id: the number of a production
/// id, or the source path of a development id with its `[project]/`-style root,
/// layer and modifiers dropped (and only the package path of a `node_modules`
/// file), e.g. `"[project]/src/foo.ts [app-client] (ecmascript)"` → `src_foo_ts`.
pub fn module_ident(id: &str) -> String {
    let path = [" [", " (", " <", " {"]
        .iter()
        .filter_map(|modifier| id.find(modifier))
        .min()
        .map_or(id, |end| &id[..end]);
    let path = path
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("]/"))
        .map_or(path, |(_, rest)| rest);
    let path = path.rsplit_once("node_modules/").map_or(path, |(_, package)| package);
    let mut ident = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_alphanumeric() || c == '$' {
            ident.push(c);
        } else if !ident.is_empty() && !ident.ends_with('_') {
            ident.push('_');
        }
    }
    while ident.ends_with('_') {
        ident.pop();
    }
    if ident.is_empty() {
        ident.push_str("anonymous");
    }
    ident
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
