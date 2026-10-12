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
//! `script` as runs of module ids, each run closed by its factory. A leading
//! array element is a nested list it installs first: a chunk may put its
//! strict factories there, inside `(() => { "use strict"; return [ … ] })()`
//! (Next.js 16.4 emits a plain `[ … ]`), or wrap the whole chunk in
//! `(() => { "use strict"; … })()` when every factory is strict
//! (`turbopack-ecmascript/src/chunk/factory_group.rs`).
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
    BindingIdentifier, BindingPattern, CallExpression, Expression, FormalParameters, FunctionBody, NewExpression,
    ObjectExpression, ObjectPropertyKind, Program, Statement,
};
use oxc_syntax::operator::{AssignmentOperator, LogicalOperator};

/// One run of module ids and the factory they share.
pub struct FactoryGroup<'b, 'a> {
    pub ids: Vec<String>,
    /// The id literals as written: `"123"` and `123` are different ids.
    pub id_spans: Vec<oxc_span::Span>,
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

/// The name of a global read as `globalThis.G`, `self["G"]`, `window.G` or `global.G`.
pub fn global_name<'b>(expr: &'b Expression<'_>) -> Option<Cow<'b, str>> {
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

/// [`global_name`] of an assignment target.
pub fn global_target_name<'b>(target: &'b AssignmentTarget<'_>) -> Option<Cow<'b, str>> {
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
    let mut id_spans = Vec::new();
    for element in elements {
        let expr = element.as_expression()?;
        if let Some(id) = module_id(expr) {
            ids.push(id);
            id_spans.push(oxc_span::GetSpan::span(expr));
        } else if !ids.is_empty() && factory_params(expr).is_some() {
            out.push(FactoryGroup {
                ids: std::mem::take(&mut ids),
                id_spans: std::mem::take(&mut id_spans),
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
        && let Some(expr) = first.as_expression()
    {
        // The runtime installs a leading array as a nested factory list
        // (`installCompressedModuleFactories`); Next.js 16.4 emits it as a
        // plain array literal, later versions as the strict IIFE.
        if let Some(strict) = strict_factory_list(expr) {
            push_groups(&strict.elements, true, &mut groups)?;
            rest = tail;
        } else if let Expression::ArrayExpression(nested) = expr.without_parentheses() {
            push_groups(&nested.elements, false, &mut groups)?;
            rest = tail;
        }
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
/// file), e.g. `"[project]/src/foo.ts [app-client] (ecmascript)"` → `src_foo_ts`,
/// plus its module parts: `"… (ecmascript) <locals>"` → `src_foo_ts_locals`.
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
    // A module part (`<locals>`, `<export foo>`, `<internal part 3>`) tells
    // the parts of one file apart.
    let tail = id.rsplit_once('}').map_or(id, |(_, tail)| tail);
    let parts = tail
        .split(" <")
        .skip(1)
        .filter_map(|part| part.split_once('>').map(|(part, _)| part));
    let mut ident = String::with_capacity(path.len());
    for c in path
        .chars()
        .chain(parts.flat_map(|part| std::iter::once(' ').chain(part.chars())))
    {
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

/// Where the bindings of one `esmReexport` group come from.
pub enum ReexportSource<'b, 'a> {
    /// A module id the runtime imports.
    Module(String),
    /// An already imported namespace value.
    Namespace(&'b Expression<'a>),
}

/// One group of `__turbopack_context__.S([…])`: a source and its
/// `(exported name, imported name)` pairs.
pub struct ReexportGroup<'b, 'a> {
    pub source: ReexportSource<'b, 'a>,
    pub pairs: Vec<(String, String)>,
}

/// The groups of `__turbopack_context__.S(list, id?)`, read like the runtime's
/// `esmReexport` (`shared/runtime/runtime-utils.ts`): each group is a head (a
/// module id or a namespace value), then `exported, imported` name pairs (or
/// one string joining them with commas), ended by `0` or the end of the list.
/// `None` unless every group reads.
pub fn reexport_groups<'b, 'a>(list: &'b ArrayExpression<'a>) -> Option<Vec<ReexportGroup<'b, 'a>>> {
    let mut groups = Vec::new();
    let mut elements = list.elements.iter();
    while let Some(head) = elements.next() {
        let head = head.as_expression()?;
        let source = module_id(head).map_or(ReexportSource::Namespace(head), ReexportSource::Module);
        let mut names = Vec::new();
        for element in elements.by_ref() {
            match element {
                ArrayExpressionElement::NumericLiteral(end) if end.value == 0.0 => break,
                ArrayExpressionElement::StringLiteral(name) => names.push(name.value.to_string()),
                _ => return None,
            }
        }
        if let [joined] = names.as_slice() {
            names = joined.split(',').map(str::to_string).collect();
        }
        if names.len() % 2 != 0 {
            return None;
        }
        let pairs = names
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[exported, imported]| (exported.clone(), imported.clone()))
            .collect();
        groups.push(ReexportGroup { source, pairs });
    }
    Some(groups)
}

/// The map of a `require.context` module, `{ "./a.js": { id: () => ID, module: () => … } }`
/// (`turbopack-ecmascript/src/references/require_context.rs`), as
/// `(key, module id)` pairs. `None` unless every property has that shape.
pub fn context_map(obj: &ObjectExpression<'_>) -> Option<Vec<(String, String)>> {
    let mut entries = Vec::with_capacity(obj.properties.len());
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(prop) = prop else {
            return None;
        };
        let key = prop.key.static_name()?;
        let Expression::ObjectExpression(entry) = prop.value.without_parentheses() else {
            return None;
        };
        let id = entry.properties.iter().find_map(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name("id") => {
                match p.value.without_parentheses() {
                    Expression::ArrowFunctionExpression(arrow) => arrow.get_expression().and_then(module_id),
                    _ => None,
                }
            }
            _ => None,
        })?;
        let has_module = entry
            .properties
            .iter()
            .any(|p| matches!(p, ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name("module")));
        if !has_module {
            return None;
        }
        entries.push((key.to_string(), id));
    }
    (!entries.is_empty()).then_some(entries)
}

/// One chunk an async loader loads.
///
/// A chunk path, or with component chunks (Next.js
/// `turbopackChunking.generateComponentChunks`) a merged chunk
/// `[url, componentChunkPaths, componentChunkSizes]`
/// (`browser/runtime/base/runtime-base.ts`, `ChunkUrlOrMerged`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LoaderChunk {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
}

/// The factory body of an async loader module
/// (`turbopack-ecmascript/src/async_chunk/module.rs`), the argument of
/// `ctx.v(…)`: `(parentImport) => Promise.all([chunk, …].map((chunk) => ctx.l(chunk))).then(() => parentImport(ID))`,
/// or `Promise.resolve().then(() => parentImport(ID))` when it loads no chunk.
/// `ctx.A(loader)` calls it with the importer's `esmImport`.
pub struct AsyncLoader<'b, 'a> {
    pub parent_import: &'b BindingIdentifier<'a>,
    pub chunks: Vec<LoaderChunk>,
    /// The module it imports once its chunks are loaded.
    pub module: Option<String>,
}

/// A function's parameters and what it returns: an arrow's expression body,
/// or the argument of a body that is a single `return`. `Some(None)` for an
/// empty body.
fn params_and_return<'b, 'a>(
    expr: &'b Expression<'a>,
) -> Option<(&'b FormalParameters<'a>, Option<&'b Expression<'a>>)> {
    let (params, body) = match expr.without_parentheses() {
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) => (&arrow.params, &**body),
            _ => return Some((&arrow.params, arrow.get_expression())),
        },
        Expression::FunctionExpression(func) => (&func.params, func.body.as_deref()?),
        _ => return None,
    };
    match body.statements.as_slice() {
        [] => Some((params, None)),
        [Statement::ReturnStatement(ret)] => Some((params, ret.argument.as_ref())),
        _ => None,
    }
}

/// See [`AsyncLoader`]; `value` is the argument of `ctx.v(…)`.
pub fn async_loader<'b, 'a>(value: &'b Expression<'a>) -> Option<AsyncLoader<'b, 'a>> {
    let (params, returned) = params_and_return(value)?;
    let [param] = params.items.as_slice() else {
        return None;
    };
    let BindingPattern::BindingIdentifier(parent_import) = &param.pattern else {
        return None;
    };
    let Expression::CallExpression(then) = returned?.without_parentheses() else {
        return None;
    };
    let Expression::StaticMemberExpression(then_callee) = then.callee.without_parentheses() else {
        return None;
    };
    if then_callee.property.name != "then" || then.arguments.len() > 1 {
        return None;
    }
    let module = match then.arguments.first() {
        None => None,
        Some(callback) => {
            let (params, returned) = params_and_return(callback.as_expression()?)?;
            if !params.items.is_empty() {
                return None;
            }
            match returned.map(Expression::without_parentheses) {
                Some(Expression::CallExpression(call)) if matches!(call.callee.without_parentheses(), Expression::Identifier(id) if id.name == parent_import.name) =>
                {
                    let [arg] = call.arguments.as_slice() else {
                        return None;
                    };
                    Some(module_id(arg.as_expression()?)?)
                }
                None => None,
                _ => return None,
            }
        }
    };
    let Expression::CallExpression(promise) = then_callee.object.without_parentheses() else {
        return None;
    };
    let Expression::StaticMemberExpression(promise_callee) = promise.callee.without_parentheses() else {
        return None;
    };
    if !matches!(&promise_callee.object, Expression::Identifier(id) if id.name == "Promise") {
        return None;
    }
    let chunks = match (promise_callee.property.name.as_str(), promise.arguments.as_slice()) {
        ("resolve", []) => Vec::new(),
        ("all", [Argument::CallExpression(map)]) => {
            let Expression::StaticMemberExpression(map_callee) = map.callee.without_parentheses() else {
                return None;
            };
            let Expression::ArrayExpression(list) = map_callee.object.without_parentheses() else {
                return None;
            };
            if map_callee.property.name != "map" {
                return None;
            }
            list.elements
                .iter()
                .map(|element| loader_chunk(element.as_expression()?))
                .collect::<Option<Vec<_>>>()?
        }
        _ => return None,
    };
    Some(AsyncLoader {
        parent_import,
        chunks,
        module,
    })
}

/// A chunk path, `{ path, … }` chunk data (development), or a merged chunk
/// `[url, [component, …], [size, …]]`.
fn loader_chunk(expr: &Expression<'_>) -> Option<LoaderChunk> {
    let strings = |expr: &Expression<'_>| -> Option<Vec<String>> {
        let Expression::ArrayExpression(list) = expr.without_parentheses() else {
            return None;
        };
        list.elements
            .iter()
            .map(|e| match e {
                ArrayExpressionElement::StringLiteral(s) => Some(s.value.to_string()),
                _ => None,
            })
            .collect()
    };
    match expr.without_parentheses() {
        Expression::StringLiteral(path) => Some(LoaderChunk {
            path: path.value.to_string(),
            components: Vec::new(),
        }),
        Expression::ObjectExpression(data) => data.properties.iter().find_map(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name("path") => match &p.value {
                Expression::StringLiteral(path) => Some(LoaderChunk {
                    path: path.value.to_string(),
                    components: Vec::new(),
                }),
                _ => None,
            },
            _ => None,
        }),
        Expression::ArrayExpression(merged) => {
            let [ArrayExpressionElement::StringLiteral(url), components, _sizes] = merged.elements.as_slice() else {
                return None;
            };
            Some(LoaderChunk {
                path: url.value.to_string(),
                components: strings(components.as_expression()?)?,
            })
        }
        _ => None,
    }
}

/// The path of a server asset module, `ctx.v("/_next/static/media/a.png" + (globalThis.NEXT_CLIENT_ASSET_SUFFIX || ""))`
/// (`ctx.q(path)` in the browser).
pub fn asset_path<'b>(value: &'b Expression<'_>) -> Option<&'b str> {
    let path = match value.without_parentheses() {
        Expression::StringLiteral(path) => path,
        Expression::BinaryExpression(concat) if concat.operator == oxc_syntax::operator::BinaryOperator::Addition => {
            match &concat.left {
                Expression::StringLiteral(path) => path,
                _ => return None,
            }
        }
        _ => return None,
    };
    path.value.contains('/').then_some(path.value.as_str())
}

/// The entry name of the Next.js edge wrapper's
/// `self._ENTRIES["middleware_<page>"] = new Proxy(modProm, { get(innerModProm, name) {…} })`
/// (`packages/next/src/build/templates/edge-wrapper.js`, emitted by
/// `crates/next-core/src/next_edge/entry.rs`), with the proxy.
pub fn edge_entry<'b, 'a>(assign: &'b AssignmentExpression<'a>) -> Option<(Cow<'b, str>, &'b NewExpression<'a>)> {
    if assign.operator != AssignmentOperator::Assign {
        return None;
    }
    let (object, name) = match &assign.left {
        AssignmentTarget::StaticMemberExpression(m) => (&m.object, Cow::Borrowed(m.property.name.as_str())),
        AssignmentTarget::ComputedMemberExpression(m) => (&m.object, Cow::from(m.static_property_name()?)),
        _ => return None,
    };
    if global_name(object).as_deref() != Some("_ENTRIES") {
        return None;
    }
    let Expression::NewExpression(proxy) = assign.right.without_parentheses() else {
        return None;
    };
    let is_proxy = matches!(&proxy.callee, Expression::Identifier(id) if id.name == "Proxy");
    (is_proxy && proxy.arguments.len() == 2).then_some((name, proxy))
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
