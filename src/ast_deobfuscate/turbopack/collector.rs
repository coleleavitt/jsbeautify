use oxc_ast::ast::{
    Argument, ArrayExpression, AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, ConditionalExpression, Expression, Function, ObjectPropertyKind, Program, SimpleAssignmentTarget,
    Statement, VariableDeclarator, WithStatement,
};
use oxc_syntax::identifier::is_identifier_name;
use oxc_syntax::keyword::is_reserved_keyword_or_global_object;
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::default_export::{DefaultExport, default_export_names};
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::webpack::shapes::{getter_target, resolve, resolve_expr, simple_params};

use super::shapes::{
    EsmBinding, Payload, Registration, chunk_list, esm_bindings, factory_params, module_id, module_ident,
    node_entry_runtime, program_node_chunk, registration,
};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

const FACTORY_PARAMS: [&str; 3] = ["__turbopack_context__", "module", "exports"];
const ASYNC_BODY_PARAMS: [&str; 2] = ["__turbopack_handle_async_dependencies__", "__turbopack_async_result__"];

/// Runtime function names by context member, from
/// `turbopack-ecmascript-runtime` (`contextPrototype.<member> = <name>`).
const RUNTIME_MEMBERS: [(&str, &str); 25] = [
    ("A", "asyncLoader"),
    ("C", "clearChunkCache"),
    ("M", "moduleFactories"),
    ("P", "resolveAbsolutePath"),
    ("R", "resolvePathFromModule"),
    ("S", "esmReexport"),
    ("U", "relativeURL"),
    ("X", "ASSET_SUFFIX"),
    ("a", "asyncModule"),
    ("b", "RUNTIME_CHUNK_BASE_PATH"),
    ("c", "moduleCache"),
    ("f", "moduleContext"),
    ("h", "getChunkRelativeUrl"),
    ("i", "esmImport"),
    ("j", "dynamicExport"),
    ("n", "exportNamespace"),
    ("q", "exportUrl"),
    ("r", "commonJsRequire"),
    ("s", "esmExport"),
    ("t", "runtimeRequire"),
    ("v", "exportValue"),
    ("w", "RUNTIME_ROOT"),
    ("x", "externalRequire"),
    ("y", "externalImport"),
    ("z", "requireStub"),
];

/// Members whose function differs between the browser and the Node.js
/// runtime: `(member, browser name, Node.js name)`.
const RUNTIME_SPECIFIC_MEMBERS: [(&str, &str, &str); 3] = [
    ("F", "placeholderFileUrl", "resolveFileUrl"),
    ("L", "loadChunkByUrl", "loadChunkAsyncByUrl"),
    ("l", "loadChunk", "loadChunkAsync"),
];

/// Members only the Node.js runtime installs (`RUNTIME_ROOT`, externals).
const NODE_RUNTIME_MEMBERS: [&str; 3] = ["w", "x", "y"];

/// Collects the desired name of every Turbopack binding, in priority order.
#[derive(Default)]
pub(super) struct NameCollector {
    /// `__turbopack_context__` parameters of the registered factories, with
    /// the factory's chunk item id.
    contexts: FxHashMap<SymbolId, String>,
    /// The runtime's `Context` constructors and their prototype aliases.
    constructors: FxHashSet<SymbolId>,
    prototypes: FxHashSet<SymbolId>,
    plan: Vec<(SymbolId, String)>,
    planned: FxHashSet<SymbolId>,
    /// Locals exported as `default`, named after every other binding.
    defaults: Vec<DefaultExport>,
    with_scopes: Vec<ScopeId>,
    /// Functions installed under a [`RUNTIME_SPECIFIC_MEMBERS`] member, named
    /// once the runtime is known.
    runtime_specific: Vec<(SymbolId, usize)>,
    node_runtime: bool,
    pub(super) found: Found,
}

/// What [`NameCollector`] recognized, for the phase log.
#[derive(Default)]
pub(super) struct Found {
    /// `TURBOPACK.push([…])` registrations and Node.js `module.exports = […]` chunks.
    pub registrations: usize,
    pub factories: usize,
    pub node_chunks: usize,
    pub chunk_lists: usize,
    pub node_entries: usize,
    pub worker_entrypoint: bool,
}

impl Found {
    pub(super) const fn is_empty(&self) -> bool {
        self.registrations == 0 && self.chunk_lists == 0 && self.node_entries == 0 && !self.worker_entrypoint
    }
}

impl NameCollector {
    /// The rename plan, plus the scopes containing a `with` statement.
    pub(super) fn into_plan(mut self) -> (Vec<(SymbolId, String)>, Vec<ScopeId>) {
        for (symbol, index) in std::mem::take(&mut self.runtime_specific) {
            let (_, browser, node) = RUNTIME_SPECIFIC_MEMBERS[index];
            self.plan(symbol, if self.node_runtime { node } else { browser });
        }
        (self.plan, self.with_scopes)
    }

    pub(super) fn has_runtime(&self) -> bool {
        !self.constructors.is_empty()
    }

    fn plan(&mut self, symbol: SymbolId, name: &str) {
        if self.planned.insert(symbol) {
            self.plan.push((symbol, name.to_string()));
        }
    }

    fn plan_params(&mut self, params: Option<Vec<SymbolId>>, names: &[&str]) -> Option<SymbolId> {
        let params = params.filter(|params| params.len() <= names.len())?;
        for (symbol, name) in params.iter().zip(names) {
            self.plan(*symbol, name);
        }
        params.first().copied()
    }

    fn context_member<'b>(&self, callee: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<&'b str> {
        let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
            return None;
        };
        resolve_expr(&member.object, ctx)
            .is_some_and(|symbol| self.contexts.contains_key(&symbol))
            .then_some(member.property.name.as_str())
    }

    /// The bindings of `__turbopack_context__.s(list, id?)`, where `id` names a
    /// module scope-hoisted into this factory; otherwise the factory's own.
    /// Mangled production keys are not names, but a `default` local still is.
    fn plan_exports(&mut self, call: &CallExpression<'_>, list: &ArrayExpression<'_>, ctx: &Ctx<'_>) {
        let hoisted = call.arguments.get(1).and_then(Argument::as_expression);
        let module = hoisted.map_or_else(|| self.factory_module(&call.callee, ctx), module_id);
        let bindings = esm_bindings(list);
        let names: Vec<&str> = bindings.iter().map(|(name, _)| *name).collect();
        let mangled = has_mangled_keys(&names);
        for (name, binding) in bindings {
            let local = match binding {
                EsmBinding::Value(value) => resolve_expr(value, ctx),
                EsmBinding::Getter(getter) => getter_target(getter).and_then(|id| resolve(id, ctx)),
            };
            let Some(symbol) = local else { continue };
            if name == "default" {
                self.defaults.push(DefaultExport {
                    local: symbol,
                    module: module.clone(),
                });
            } else if !mangled && is_recoverable_export_name(name) {
                self.plan(symbol, name);
            }
        }
    }

    /// The chunk item id of the factory whose context `callee` is a member of.
    fn factory_module(&self, callee: &Expression<'_>, ctx: &Ctx<'_>) -> Option<String> {
        let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
            return None;
        };
        resolve_expr(&member.object, ctx).and_then(|symbol| self.contexts.get(&symbol).cloned())
    }

    /// `x = __turbopack_context__.i(123)` / `.r(123)` → `module_123`, with a
    /// development id `"[project]/src/foo.ts [app-client] (ecmascript)"` →
    /// `module_src_foo_ts` (the id stays in the call); `x = [await]
    /// __turbopack_context__.x("pkg", …)` / `.y("pkg")` → `external_pkg`.
    fn import_name(&self, init: &Expression<'_>, ctx: &Ctx<'_>) -> Option<String> {
        let init = match init.without_parentheses() {
            Expression::AwaitExpression(expr) => &expr.argument,
            init => init,
        };
        let Expression::CallExpression(call) = init.without_parentheses() else {
            return None;
        };
        let prefix = match self.context_member(&call.callee, ctx)? {
            "i" | "r" => {
                if call.arguments.len() != 1 {
                    return None;
                }
                "module"
            }
            "x" | "y" => "external",
            _ => return None,
        };
        let id = module_id(call.arguments.first()?.as_expression()?)?;
        Some(format!("{prefix}_{}", module_ident(&id)))
    }

    fn plan_factories(&mut self, registration: Registration<'_, '_>) {
        self.found.registrations += 1;
        if let Payload::Factories(groups) = registration.payload {
            for group in groups {
                self.found.factories += 1;
                let params = factory_params(group.factory).and_then(simple_params);
                if let Some(context) = self.plan_params(params, &FACTORY_PARAMS)
                    && let Some(id) = group.ids.into_iter().next()
                {
                    self.contexts.insert(context, id);
                }
            }
        }
    }

    /// `module.exports = (sourcePath) => ({ m: (id) => getOrInstantiateRuntimeModule(sourcePath, id),
    /// c: (chunkData) => loadRuntimeChunk(sourcePath, chunkData) })`: what a Node.js entry's
    /// `require("…[turbopack]_runtime.js")` returns (`nodejs/runtime/build-base.ts`, `dev-base.ts`).
    fn plan_node_runtime_export(&mut self, assign: &AssignmentExpression<'_>, ctx: &Ctx<'_>) -> bool {
        let AssignmentTarget::StaticMemberExpression(target) = &assign.left else {
            return false;
        };
        if target.property.name != "exports"
            || !matches!(&target.object, Expression::Identifier(id) if id.name == "module")
        {
            return false;
        }
        let Expression::ArrowFunctionExpression(export) = assign.right.without_parentheses() else {
            return false;
        };
        let Some(Expression::ObjectExpression(api)) = export.body.as_expression().map(Expression::without_parentheses)
        else {
            return false;
        };
        let Some(&[source_path]) = simple_params(&export.params).as_deref() else {
            return false;
        };
        let mut found = false;
        for prop in &api.properties {
            let ObjectPropertyKind::ObjectProperty(prop) = prop else {
                continue;
            };
            let name = if prop.key.is_specific_static_name("m") {
                "getOrInstantiateRuntimeModule"
            } else if prop.key.is_specific_static_name("c") {
                "loadRuntimeChunk"
            } else {
                continue;
            };
            let Expression::ArrowFunctionExpression(method) = prop.value.without_parentheses() else {
                continue;
            };
            if let Some(Expression::CallExpression(call)) =
                method.body.as_expression().map(Expression::without_parentheses)
                && call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .and_then(|a| resolve_expr(a, ctx))
                    == Some(source_path)
                && let Some(symbol) = resolve_expr(&call.callee, ctx)
            {
                self.plan(symbol, name);
                found = true;
            }
        }
        if found {
            self.node_runtime = true;
            self.plan(source_path, "sourcePath");
        }
        found
    }

    /// `Object.assign(self, { TURBOPACK_NEXT_CHUNK_URLS: chunkUrls, TURBOPACK_ASSET_SUFFIX: param(1), … })`
    /// in the worker entrypoint (`turbopack-browser/src/ecmascript/worker.rs`).
    fn plan_worker_globals(&mut self, call: &CallExpression<'_>, ctx: &Ctx<'_>) {
        let [Argument::Identifier(target), Argument::ObjectExpression(globals)] = call.arguments.as_slice() else {
            return;
        };
        let Expression::StaticMemberExpression(callee) = call.callee.without_parentheses() else {
            return;
        };
        if callee.property.name != "assign"
            || !matches!(&callee.object, Expression::Identifier(object) if object.name == "Object")
            || target.name != "self"
        {
            return;
        }
        let mut found = false;
        for prop in &globals.properties {
            let ObjectPropertyKind::ObjectProperty(prop) = prop else {
                continue;
            };
            if prop.key.is_specific_static_name("TURBOPACK_NEXT_CHUNK_URLS") {
                found = true;
                if let Some(symbol) = resolve_expr(&prop.value, ctx) {
                    self.plan(symbol, "chunkUrls");
                }
            } else if prop.key.is_specific_static_name("TURBOPACK_ASSET_SUFFIX")
                && let Expression::CallExpression(param) = prop.value.without_parentheses()
                && let Some(symbol) = resolve_expr(&param.callee, ctx)
            {
                self.plan(symbol, "param");
            }
        }
        self.found.worker_entrypoint |= found;
    }

    fn is_prototype(&self, expr: &Expression<'_>, ctx: &Ctx<'_>) -> bool {
        match expr.without_parentheses() {
            Expression::Identifier(id) => resolve(id, ctx).is_some_and(|symbol| self.prototypes.contains(&symbol)),
            Expression::StaticMemberExpression(member) => {
                member.property.name == "prototype"
                    && resolve_expr(&member.object, ctx).is_some_and(|symbol| self.constructors.contains(&symbol))
            }
            _ => false,
        }
    }
}

/// What the worker entrypoint's `abort` is called with.
const WORKER_ABORT_MESSAGES: [&str; 2] = [
    "Worker entrypoint must be loaded in a worker context",
    "Missing worker bootstrap config",
];

/// `(n) => typeof params[n] === "string" ? params[n] : ""`: the worker
/// entrypoint's `param`; returns `params`.
fn worker_param_source(init: &Expression<'_>, ctx: &Ctx<'_>) -> Option<SymbolId> {
    let Expression::ArrowFunctionExpression(arrow) = init.without_parentheses() else {
        return None;
    };
    let params = simple_params(&arrow.params)?;
    let [param] = params.as_slice() else {
        return None;
    };
    let Some(Expression::ConditionalExpression(cond)) = arrow.body.as_expression().map(Expression::without_parentheses)
    else {
        return None;
    };
    let ConditionalExpression {
        test,
        consequent,
        alternate,
        ..
    } = &**cond;
    let indexed = |expr: &Expression<'_>| match expr.without_parentheses() {
        Expression::ComputedMemberExpression(m) if resolve_expr(&m.expression, ctx) == Some(*param) => {
            resolve_expr(&m.object, ctx)
        }
        _ => None,
    };
    let typeof_indexed = |expr: &Expression<'_>| match expr.without_parentheses() {
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::Typeof => indexed(&u.argument),
        _ => None,
    };
    let is_string = |expr: &Expression<'_>, value: &str| matches!(expr.without_parentheses(), Expression::StringLiteral(s) if s.value == value);
    let Expression::BinaryExpression(test) = test.without_parentheses() else {
        return None;
    };
    let checked = if is_string(&test.right, "string") {
        typeof_indexed(&test.left)
    } else if is_string(&test.left, "string") {
        typeof_indexed(&test.right)
    } else {
        None
    };
    let source = indexed(consequent)?;
    (matches!(test.operator, BinaryOperator::StrictEquality | BinaryOperator::Equality)
        && is_string(alternate, "")
        && checked == Some(source))
    .then_some(source)
}

/// Production builds may replace export names with the shortest keys that fit
/// them all (`turbopack-ecmascript/src/references/esm/mangle/table.rs`): one
/// character for up to 54 exports, two for up to 3510. Such keys are not names.
fn has_mangled_keys(names: &[&str]) -> bool {
    let width = match names.len() {
        1..=54 => 1,
        55..=3510 => 2,
        _ => return false,
    };
    names.iter().all(|name| name.len() <= width)
}

/// Export names worth giving to a local: legal binding names other than `default`.
fn is_recoverable_export_name(name: &str) -> bool {
    name != "default"
        && is_identifier_name(name)
        && !is_reserved_keyword_or_global_object(name)
        && !matches!(name, "arguments" | "eval")
}

/// `this.<prop> = <param>` resolving to `param`.
fn this_assignment<'b>(expr: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<(&'b str, SymbolId)> {
    let Expression::AssignmentExpression(assign) = expr else {
        return None;
    };
    let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign || !matches!(member.object, Expression::ThisExpression(_)) {
        return None;
    }
    Some((member.property.name.as_str(), resolve_expr(&assign.right, ctx)?))
}

/// `function Context(module, exports) { this.m = module; this.e = exports }`.
fn is_context_constructor(func: &Function<'_>, ctx: &Ctx<'_>) -> bool {
    let params = simple_params(&func.params);
    let (Some(body), Some([module, exports])) = (func.body.as_ref(), params.as_deref()) else {
        return false;
    };
    let mut assignments = Vec::new();
    for statement in &body.statements {
        let Statement::ExpressionStatement(stmt) = statement else {
            return false;
        };
        match &stmt.expression {
            Expression::SequenceExpression(seq) => assignments.extend(seq.expressions.iter()),
            expr => assignments.push(expr),
        }
    }
    let assigned: Option<Vec<_>> = assignments.into_iter().map(|e| this_assignment(e, ctx)).collect();
    matches!(assigned.as_deref(), Some([("m", m), ("e", e)]) if m == module && e == exports)
}

impl<'a> Traverse<'a, DeobfuscateState> for NameCollector {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let defaults = std::mem::take(&mut self.defaults);
        for (symbol, name) in default_export_names(program, ctx.scoping(), defaults) {
            self.plan(symbol, &name);
        }
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get())
            && is_context_constructor(func, ctx)
        {
            self.constructors.insert(symbol);
            self.plan(symbol, "Context");
        }
    }

    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(registration) = program_node_chunk(program) {
            self.found.node_chunks += 1;
            self.plan_factories(registration);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if let Some(registration) = registration(call) {
            self.plan_factories(registration);
            return;
        }
        if chunk_list(call).is_some() {
            self.found.chunk_lists += 1;
            return;
        }
        self.plan_worker_globals(call, ctx);
        if let [Argument::StringLiteral(message)] = call.arguments.as_slice()
            && WORKER_ABORT_MESSAGES.contains(&message.value.as_str())
            && let Some(symbol) = resolve_expr(&call.callee, ctx)
        {
            self.plan(symbol, "abort");
        }

        match self.context_member(&call.callee, ctx) {
            Some("s") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first() {
                    self.plan_exports(call, list, ctx);
                }
            }
            Some("a") => {
                let body = call.arguments.first().and_then(Argument::as_expression);
                let params = match body.map(Expression::without_parentheses) {
                    Some(Expression::ArrowFunctionExpression(arrow)) if arrow.r#async => simple_params(&arrow.params),
                    Some(Expression::FunctionExpression(func)) if func.r#async && !func.generator => {
                        simple_params(&func.params)
                    }
                    _ => None,
                };
                self.plan_params(params, &ASYNC_BODY_PARAMS);
            }
            _ => {}
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let (Some(symbol), Some(init)) = (id.symbol_id.get(), decl.init.as_ref()) else {
            return;
        };
        if let Some(name) = self.import_name(init, ctx) {
            self.plan(symbol, &name);
        } else if node_entry_runtime(init).is_some() {
            self.found.node_entries += 1;
        } else if let Some(params) = worker_param_source(init, ctx) {
            self.plan(symbol, "param");
            self.plan(params, "params");
        } else if !self.constructors.is_empty() && self.is_prototype(init, ctx) {
            self.prototypes.insert(symbol);
            self.plan(symbol, "contextPrototype");
        }
    }

    /// `contextPrototype.i = esmImport` in the runtime chunk, and the Node.js
    /// runtime's export.
    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        if self.constructors.is_empty() || assign.operator != AssignmentOperator::Assign {
            return;
        }
        if self.plan_node_runtime_export(assign, ctx) {
            return;
        }
        let Some(SimpleAssignmentTarget::StaticMemberExpression(member)) = assign.left.as_simple_assignment_target()
        else {
            return;
        };
        if !self.is_prototype(&member.object, ctx) {
            return;
        }
        let member_name = member.property.name.as_str();
        self.node_runtime |= NODE_RUNTIME_MEMBERS.contains(&member_name);
        let Some(symbol) = resolve_expr(&assign.right, ctx) else {
            return;
        };
        if let Some((_, name)) = RUNTIME_MEMBERS.iter().find(|(m, _)| *m == member_name) {
            self.plan(symbol, name);
        } else if let Some(index) = RUNTIME_SPECIFIC_MEMBERS.iter().position(|(m, ..)| *m == member_name) {
            self.runtime_specific.push((symbol, index));
        }
    }

    fn enter_with_statement(&mut self, _stmt: &mut WithStatement<'a>, ctx: &mut Ctx<'a>) {
        self.with_scopes.push(ctx.current_scope_id());
    }
}
