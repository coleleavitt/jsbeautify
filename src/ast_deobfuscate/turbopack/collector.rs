use oxc_ast::ast::{
    Argument, ArrayExpression, AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, Class, ClassElement, ConditionalExpression, Expression, Function, ObjectExpression,
    ObjectPropertyKind, Program, PropertyKey, SimpleAssignmentTarget, Statement, StaticMemberExpression, StringLiteral,
    TemplateLiteral, VariableDeclarator, WithStatement,
};
use oxc_syntax::identifier::is_identifier_name;
use oxc_syntax::keyword::is_reserved_keyword_or_global_object;
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::default_export::{DefaultExport, default_export_names, is_minified};
use crate::ast_deobfuscate::project::{TurbopackFacts, TurbopackKeyName, TurbopackMangledExport, TurbopackReexport};
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::webpack::shapes::{getter_target, resolve, resolve_expr, simple_params};

use super::shapes::{
    EsmBinding, Payload, ReexportSource, Registration, chunk_list, edge_entry, esm_bindings, factory_params,
    global_name, global_target_name, module_id, module_ident, node_entry_runtime, program_node_chunk, reexport_groups,
    registration,
};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

const FACTORY_PARAMS: [&str; 3] = ["__turbopack_context__", "module", "exports"];
const ASYNC_BODY_PARAMS: [&str; 2] = ["__turbopack_handle_async_dependencies__", "__turbopack_async_result__"];

/// Runtime function names by context member, from
/// `turbopack-ecmascript-runtime` (`contextPrototype.<member> = <name>`).
const RUNTIME_MEMBERS: [(&str, &str); 24] = [
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

/// Members whose function depends on the runtime, named once it is known.
const RUNTIME_SPECIFIC_MEMBERS: [&str; 4] = ["F", "L", "c", "l"];

/// Members only the Node.js runtime installs (`RUNTIME_ROOT`, externals).
const NODE_RUNTIME_MEMBERS: [&str; 3] = ["w", "x", "y"];

/// The runtime-specific name of the function installed as `member`, from
/// `browser/runtime/base/*.ts`, `nodejs/runtime/*.ts` and their `dev-base.ts`.
/// A development Node.js runtime installs `l` twice; the later
/// (`override_`) one loads chunks on demand.
fn runtime_member_name(member: &str, node: bool, dev: bool, override_: bool) -> Option<&'static str> {
    Some(match (member, node) {
        ("F", false) => "placeholderFileUrl",
        ("F", true) => "resolveFileUrl",
        ("L", false) => "loadChunkByUrl",
        ("L", true) => "loadChunkAsyncByUrl",
        ("c", _) if dev => "devModuleCache",
        ("c", _) => "moduleCache",
        ("l", false) => "loadChunk",
        ("l", true) if dev && override_ => "loadChunkAsyncOnDemand",
        ("l", true) => "loadChunkAsync",
        _ => return None,
    })
}

/// Runtime functions by a string the runtime source writes only inside them
/// (template literals as their quasis joined by `${}`), from
/// `turbopack-ecmascript-runtime/js/src/**` (Next.js 16.4 output). They name
/// the minified functions of a production runtime and of the development
/// runtime's HMR code.
const RUNTIME_FUNCTION_ANCHORS: [(&str, &str); 19] = [
    ("Unknown chunk update type: ${}", "applyChunkListUpdate"),
    ("Unknown merged chunk update type", "computeChangedModules"),
    (
        "cannot apply update: unaccepted module. ${}.",
        "computedInvalidatedModules",
    ),
    ("invalid mode: ${}", "disposeModule"),
    ("can't infer type of chunk from URL ${}", "doLoadChunk"),
    (
        "[Server HMR] No update handler registered to receive message",
        "emitMessage",
    ),
    ("unexpected tag: ${}", "esm"),
    (
        "Module ${} was instantiated ${}, but the module factory is not available.",
        "factoryNotAvailableMessage",
    ),
    ("Dependency chain: ${}", "formatDependencyChain"),
    ("chunk path empty but not in a worker", "getChunkFromRegistration"),
    ("[Server HMR] Update failed, full reload needed:", "handleNodejsUpdate"),
    (
        "[Server HMR] Server HMR client is already initialized",
        "initializeServerHmr",
    ),
    (
        "malformed chunk format, expected a factory function",
        "installCompressedModuleFactories",
    ),
    ("Invariant: ${}", "invariant"),
    ("Failed to load chunk ${} from module ${}", "loadChunkAsync"),
    ("Failed to load chunk ${} ${}${}", "loadChunkByUrlWhole"),
    (" from runtime for chunk ${}", "loadRuntimeChunkPath"),
    ("Refusing to load script from foreign origin: ", "sameOriginUrl"),
    (
        "As an immediate workaround, try hard-reloading the page, clearing the browser cache, or unregistering any service workers.",
        "instantiateModuleShared",
    ),
];

/// Collects the desired name of every Turbopack binding, in priority order.
#[derive(Default)]
pub(super) struct NameCollector {
    /// `__turbopack_context__` parameters of the registered factories, with
    /// the factory's module ids.
    contexts: FxHashMap<SymbolId, Vec<String>>,
    /// The runtime's `Context` constructors and their prototype aliases.
    constructors: FxHashSet<SymbolId>,
    prototypes: FxHashSet<SymbolId>,
    plan: Vec<(SymbolId, String)>,
    planned: FxHashSet<SymbolId>,
    /// Locals exported as `default`, named after every other binding.
    defaults: Vec<DefaultExport>,
    with_scopes: Vec<ScopeId>,
    /// Functions installed under a [`RUNTIME_SPECIFIC_MEMBERS`] member, in
    /// order, named once the runtime is known.
    runtime_specific: Vec<(SymbolId, &'static str)>,
    node_runtime: bool,
    dev_runtime: bool,
    /// Runtime-only candidates (`(symbol, name)`), kept when the runtime is
    /// found: [`RUNTIME_FUNCTION_ANCHORS`] and error classes.
    runtime_candidates: Vec<(SymbolId, &'static str)>,
    runtime_error_classes: Vec<(SymbolId, String)>,
    /// Enclosing function declarations; `None` for other `function`s.
    functions: Vec<Option<SymbolId>>,
    /// `registerChunk` / `registerChunkList` and the name of the list each
    /// registers with `list.forEach(fn)`.
    register_fns: FxHashMap<SymbolId, &'static str>,
    /// Module ids loaded into locals with `__turbopack_context__.i/r(id)`.
    import_modules: FxHashMap<SymbolId, String>,
    /// `(module id, export key)` → export name, from `esmReexport` lists; a
    /// key read under two different names maps to `None`.
    reexport_names: FxHashMap<(String, String), Option<String>>,
    /// Locals exported under a mangled key: `(module ids, key, local)`.
    mangled_exports: Vec<(Vec<String>, String, SymbolId)>,
    /// `var _s = __turbopack_context__.k.signature()` handles not yet named.
    refresh_signatures: FxHashSet<SymbolId>,
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
    pub edge_entries: usize,
    pub worker_entrypoint: bool,
}

impl Found {
    pub(super) const fn is_empty(&self) -> bool {
        self.registrations == 0
            && self.chunk_lists == 0
            && self.node_entries == 0
            && self.edge_entries == 0
            && !self.worker_entrypoint
    }
}

impl NameCollector {
    /// The rename plan, plus the scopes containing a `with` statement.
    pub(super) fn into_plan(mut self) -> (Vec<(SymbolId, String)>, Vec<ScopeId>) {
        let specific = std::mem::take(&mut self.runtime_specific);
        for (index, &(symbol, member)) in specific.iter().enumerate() {
            let override_ = specific[..index].iter().any(|(_, m)| *m == member);
            if let Some(name) = runtime_member_name(member, self.node_runtime, self.dev_runtime, override_) {
                self.plan(symbol, name);
            }
        }
        if self.has_runtime() {
            let candidates = std::mem::take(&mut self.runtime_candidates);
            for &(symbol, name) in &candidates {
                let unique = candidates
                    .iter()
                    .all(|&(s, n)| (s == symbol) == (n == name) || (s != symbol && n != name));
                if unique {
                    self.plan(symbol, name);
                }
            }
            for (symbol, name) in std::mem::take(&mut self.runtime_error_classes) {
                self.plan(symbol, &name);
            }
        }
        for (ids, key, symbol) in std::mem::take(&mut self.mangled_exports) {
            let name = ids
                .iter()
                .find_map(|id| self.reexport_names.get(&(id.clone(), key.clone())).cloned().flatten());
            if let Some(name) = name {
                self.plan(symbol, &name);
            }
        }
        (self.plan, self.with_scopes)
    }

    /// Export names other files' `esmReexport` lists give module keys (see
    /// [`crate::ast_deobfuscate::project`]). The lists of this file still
    /// apply, and a key named differently here is not named.
    pub(super) fn seed_reexports(&mut self, reexports: &[TurbopackReexport]) {
        for reexport in reexports {
            self.reexport_names.insert(
                (reexport.module.clone(), reexport.key.clone()),
                Some(reexport.name.clone()),
            );
        }
    }

    /// What other files need to know about this one: the module ids of its
    /// factories, its `esmReexport` names, and its mangled export keys.
    pub(super) fn project_facts(&self) -> TurbopackFacts {
        let mut module_ids: Vec<String> = self.contexts.values().flatten().cloned().collect();
        module_ids.sort_unstable();
        module_ids.dedup();
        let mut reexports: Vec<_> = self
            .reexport_names
            .iter()
            .map(|((module, key), name)| TurbopackKeyName {
                module: module.clone(),
                key: key.clone(),
                name: name.clone(),
            })
            .collect();
        reexports.sort_unstable_by(|a, b| (&a.module, &a.key).cmp(&(&b.module, &b.key)));
        let mangled_exports = self
            .mangled_exports
            .iter()
            .map(|(modules, key, _)| TurbopackMangledExport {
                modules: modules.clone(),
                key: key.clone(),
            })
            .collect();
        TurbopackFacts {
            module_ids,
            reexports,
            mangled_exports,
        }
    }

    pub(super) fn has_runtime(&self) -> bool {
        !self.constructors.is_empty()
    }

    /// Plan a runtime name for `symbol` unless it already carries an
    /// authored (unminified) name, such as `devContextPrototype`.
    fn plan_runtime(&mut self, symbol: SymbolId, name: &str, ctx: &Ctx<'_>) {
        if is_minified(ctx.scoping().symbol_name(symbol)) {
            self.plan(symbol, name);
        }
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

    fn is_context(&self, expr: &Expression<'_>, ctx: &Ctx<'_>) -> bool {
        resolve_expr(expr, ctx).is_some_and(|symbol| self.contexts.contains_key(&symbol))
    }

    fn context_member<'b>(&self, callee: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<&'b str> {
        let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
            return None;
        };
        self.is_context(&member.object, ctx)
            .then_some(member.property.name.as_str())
    }

    /// `register` / `signature` in `__turbopack_context__.k.<method>(…)`: React
    /// Refresh, which `turbopack-ecmascript/src/transform/mod.rs` configures
    /// as SWC's `refresh_reg` / `refresh_sig` in development.
    fn refresh_method<'b>(&self, callee: &'b Expression<'_>, ctx: &Ctx<'_>) -> Option<&'b str> {
        let Expression::StaticMemberExpression(method) = callee.without_parentheses() else {
            return None;
        };
        let Expression::StaticMemberExpression(refresh) = method.object.without_parentheses() else {
            return None;
        };
        (refresh.property.name == "k" && self.is_context(&refresh.object, ctx)).then_some(method.property.name.as_str())
    }

    /// The bindings of `__turbopack_context__.s(list, id?)`, where `id` names a
    /// module scope-hoisted into this factory; otherwise the factory's own.
    /// Mangled production keys are not names, but a `default` local still is,
    /// and an `esmReexport` elsewhere in the file may name the key.
    fn plan_exports(&mut self, call: &CallExpression<'_>, list: &ArrayExpression<'_>, ctx: &Ctx<'_>) {
        let modules = self.export_modules(call, ctx);
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
                    module: modules.first().cloned(),
                });
            } else if !mangled && is_recoverable_export_name(name) {
                self.plan(symbol, name);
            } else if mangled && !modules.is_empty() {
                self.mangled_exports.push((modules.clone(), name.to_string(), symbol));
            }
        }
    }

    /// The module ids a `s` / `S` call declares exports for: its hoisted id
    /// argument, or the ids of the factory whose context it calls.
    fn export_modules(&self, call: &CallExpression<'_>, ctx: &Ctx<'_>) -> Vec<String> {
        if let Some(id) = call
            .arguments
            .get(1)
            .and_then(Argument::as_expression)
            .and_then(module_id)
        {
            return vec![id];
        }
        let Expression::StaticMemberExpression(member) = call.callee.without_parentheses() else {
            return Vec::new();
        };
        resolve_expr(&member.object, ctx)
            .and_then(|symbol| self.contexts.get(&symbol).cloned())
            .unwrap_or_default()
    }

    /// `__turbopack_context__.S([id | namespace, "exported,imported,…", 0, …])`
    /// tells the export name behind another module's (possibly mangled) key.
    fn record_reexports(&mut self, list: &ArrayExpression<'_>, ctx: &Ctx<'_>) {
        let Some(groups) = reexport_groups(list) else {
            return;
        };
        let exported: Vec<&str> = groups
            .iter()
            .flat_map(|group| group.pairs.iter().map(|(name, _)| name.as_str()))
            .collect();
        if has_mangled_keys(&exported) {
            return;
        }
        for group in &groups {
            let source = match &group.source {
                ReexportSource::Module(id) => Some(id.clone()),
                ReexportSource::Namespace(expr) => {
                    resolve_expr(expr, ctx).and_then(|symbol| self.import_modules.get(&symbol).cloned())
                }
            };
            let Some(source) = source else { continue };
            for (name, key) in &group.pairs {
                if !is_recoverable_export_name(name) {
                    continue;
                }
                self.reexport_names
                    .entry((source.clone(), key.clone()))
                    .and_modify(|known| {
                        if known.as_deref() != Some(name.as_str()) {
                            *known = None;
                        }
                    })
                    .or_insert_with(|| Some(name.clone()));
            }
        }
    }

    /// `x = __turbopack_context__.i(123)` / `.r(123)` → `module_123`, with a
    /// development id `"[project]/src/foo.ts [app-client] (ecmascript)"` →
    /// `module_src_foo_ts` (the id stays in the call); `x = [await]
    /// __turbopack_context__.x("pkg", …)` / `.y("pkg")` → `external_pkg`;
    /// `x = __turbopack_context__.f(__turbopack_context__.r(id))` (a
    /// `require.context` map) → `requireContext_<id>`. Returns the name and
    /// the module id an `.i` / `.r` import loads.
    fn import_name(&self, init: &Expression<'_>, ctx: &Ctx<'_>) -> Option<(String, Option<String>)> {
        let init = match init.without_parentheses() {
            Expression::AwaitExpression(expr) => &expr.argument,
            init => init,
        };
        let Expression::CallExpression(call) = init.without_parentheses() else {
            return None;
        };
        let member = self.context_member(&call.callee, ctx)?;
        let arg = call.arguments.first()?.as_expression()?;
        match member {
            "i" | "r" if call.arguments.len() == 1 => {
                let id = module_id(arg)?;
                Some((format!("module_{}", module_ident(&id)), Some(id)))
            }
            "x" | "y" => Some((format!("external_{}", module_ident(&module_id(arg)?)), None)),
            "f" if call.arguments.len() == 1 => {
                let map = match arg.without_parentheses() {
                    Expression::CallExpression(map) if self.context_member(&map.callee, ctx) == Some("r") => {
                        map.arguments.first()?.as_expression().and_then(module_id)
                    }
                    Expression::ObjectExpression(_) => None,
                    _ => return None,
                };
                let name = map.map_or_else(
                    || "requireContext".to_string(),
                    |id| format!("requireContext_{}", module_ident(&id)),
                );
                Some((name, None))
            }
            _ => None,
        }
    }

    fn plan_factories(&mut self, registration: Registration<'_, '_>) {
        self.found.registrations += 1;
        if let Payload::Factories(groups) = registration.payload {
            for group in groups {
                self.found.factories += 1;
                let params = factory_params(group.factory).and_then(simple_params);
                if let Some(context) = self.plan_params(params, &FACTORY_PARAMS)
                    && !group.ids.is_empty()
                {
                    self.contexts.insert(context, group.ids);
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

    /// `self._ENTRIES["middleware_<page>"] = new Proxy(modProm, { get(innerModProm, name) {…} })`.
    fn plan_edge_entry(&mut self, assign: &AssignmentExpression<'_>, ctx: &Ctx<'_>) -> bool {
        let Some((_, proxy)) = edge_entry(assign) else {
            return false;
        };
        self.found.edge_entries += 1;
        if let Some(symbol) = proxy.arguments[0].as_expression().and_then(|e| resolve_expr(e, ctx)) {
            self.plan(symbol, "modProm");
        }
        if let Some(Expression::ObjectExpression(handler)) =
            proxy.arguments[1].as_expression().map(Expression::without_parentheses)
            && let Some(get) = object_method(handler, "get")
        {
            self.plan_params(simple_params(&get.params), &["innerModProm", "name"]);
        }
        true
    }

    /// `G = { push: registerChunk }` for the chunk loading global (and
    /// `{G}_CHUNK_LISTS = { push: registerChunkList }`) at the end of the
    /// browser runtime (`turbopack-ecmascript-runtime/src/browser_runtime.rs`).
    fn plan_registry_push(&mut self, assign: &AssignmentExpression<'_>, ctx: &Ctx<'_>) {
        let Some(global) = global_target_name(&assign.left) else {
            return;
        };
        let Expression::ObjectExpression(object) = assign.right.without_parentheses() else {
            return;
        };
        let [ObjectPropertyKind::ObjectProperty(push)] = object.properties.as_slice() else {
            return;
        };
        if !push.key.is_specific_static_name("push") {
            return;
        }
        let Some(symbol) = resolve_expr(&push.value, ctx).filter(|&s| is_minified(ctx.scoping().symbol_name(s))) else {
            return;
        };
        let (name, list) = if global.ends_with("_CHUNK_LISTS") {
            ("registerChunkList", "chunkListsToRegister")
        } else if global.starts_with("TURBOPACK") && !global.contains("_CHUNK_") {
            ("registerChunk", "chunksToRegister")
        } else {
            return;
        };
        self.plan(symbol, name);
        self.register_fns.insert(symbol, list);
    }

    /// Runtime singletons recognized by their shape: `BACKEND` /
    /// `DEV_BACKEND` (`browser/runtime/dom/*-backend-dom.ts`), the
    /// `CHUNK_UPDATE_LISTENERS` queue and the dummy refresh context.
    fn singleton_name(init: &Expression<'_>) -> Option<&'static str> {
        let Expression::ObjectExpression(object) = init.without_parentheses() else {
            return None;
        };
        let has = |name: &str| object_property(object, name).is_some();
        if has("registerChunk") && has("loadChunkCached") {
            return Some("BACKEND");
        }
        if has("unloadChunk") && has("reloadChunk") {
            return Some("DEV_BACKEND");
        }
        if has("register") && has("signature") && has("registerExports") {
            return Some("DUMMY_REFRESH_CONTEXT");
        }
        let [ObjectPropertyKind::ObjectProperty(push)] = object.properties.as_slice() else {
            return None;
        };
        let Expression::ArrowFunctionExpression(arrow) = push.value.without_parentheses() else {
            return None;
        };
        let Some(Expression::CallExpression(call)) = arrow.get_expression().map(Expression::without_parentheses) else {
            return None;
        };
        let Expression::StaticMemberExpression(queue) = call.callee.without_parentheses() else {
            return None;
        };
        (push.key.is_specific_static_name("push")
            && queue.property.name == "push"
            && global_name(&queue.object).is_some_and(|g| g.ends_with("_CHUNK_UPDATE_LISTENERS")))
        .then_some("CHUNK_UPDATE_LISTENERS")
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

    /// `function DevContext(module, exports, refresh) { Context.call(this, module, exports); this.k = refresh }`
    /// (`browser/runtime/base/dev-base.ts`).
    fn dev_context_refresh(&self, func: &Function<'_>, ctx: &Ctx<'_>) -> Option<SymbolId> {
        let params = simple_params(&func.params)?;
        let [module, exports, refresh] = params.as_slice() else {
            return None;
        };
        let mut expressions = Vec::new();
        for statement in &func.body.as_ref()?.statements {
            let Statement::ExpressionStatement(stmt) = statement else {
                return None;
            };
            match &stmt.expression {
                Expression::SequenceExpression(seq) => expressions.extend(seq.expressions.iter()),
                expr => expressions.push(expr),
            }
        }
        let [Expression::CallExpression(call), assignment] = expressions.as_slice() else {
            return None;
        };
        let Expression::StaticMemberExpression(callee) = call.callee.without_parentheses() else {
            return None;
        };
        let args: Option<Vec<_>> = call.arguments.iter().map(Argument::as_expression).collect();
        let calls_context = callee.property.name == "call"
            && resolve_expr(&callee.object, ctx).is_some_and(|symbol| self.constructors.contains(&symbol))
            && matches!(args.as_deref(), Some([Expression::ThisExpression(_), m, e])
                if resolve_expr(m, ctx) == Some(*module) && resolve_expr(e, ctx) == Some(*exports));
        (calls_context && this_assignment(assignment, ctx) == Some(("k", *refresh))).then_some(*refresh)
    }

    fn anchor(&mut self, text: &str) {
        if let Some(Some(symbol)) = self.functions.last()
            && let Some((_, name)) = RUNTIME_FUNCTION_ANCHORS.iter().find(|(anchor, _)| *anchor == text)
        {
            self.runtime_candidates.push((*symbol, name));
        }
    }
}

fn object_property<'b, 'a>(object: &'b ObjectExpression<'a>, name: &str) -> Option<&'b Expression<'a>> {
    object.properties.iter().find_map(|prop| match prop {
        ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name(name) => Some(&p.value),
        _ => None,
    })
}

/// The function of `name(…) {…}` / `name: function (…) {…}` in `object`.
fn object_method<'b, 'a>(object: &'b ObjectExpression<'a>, name: &str) -> Option<&'b Function<'a>> {
    match object_property(object, name)?.without_parentheses() {
        Expression::FunctionExpression(func) => Some(func),
        _ => None,
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

/// `class X extends Error { name = "UpdateApplyError"; … }`: the name the
/// class gives its errors.
fn error_class_name<'b>(class: &'b Class<'_>) -> Option<&'b str> {
    if !matches!(class.heritage.as_ref().map(|h| &h.expression), Some(Expression::Identifier(id)) if id.name == "Error")
    {
        return None;
    }
    class.body.body.iter().find_map(|element| match element {
        ClassElement::PropertyDefinition(prop)
            if !prop.r#static && matches!(&prop.key, PropertyKey::StaticIdentifier(key) if key.name == "name") =>
        {
            match prop.value.as_ref()?.without_parentheses() {
                Expression::StringLiteral(name) if is_identifier_name(&name.value) => Some(name.value.as_str()),
                _ => None,
            }
        }
        _ => None,
    })
}

impl<'a> Traverse<'a, DeobfuscateState> for NameCollector {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let defaults = std::mem::take(&mut self.defaults);
        for (symbol, name) in default_export_names(program, ctx.scoping(), defaults) {
            self.plan(symbol, &name);
        }
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        let symbol = func.id.as_ref().and_then(|id| id.symbol_id.get());
        let anchored = func.is_declaration() && func.id.as_ref().is_some_and(|id| is_minified(&id.name));
        self.functions.push(symbol.filter(|_| anchored));
        let Some(symbol) = symbol else { return };
        if is_context_constructor(func, ctx) {
            self.constructors.insert(symbol);
            self.plan(symbol, "Context");
        } else if !self.constructors.is_empty()
            && let Some(refresh) = self.dev_context_refresh(func, ctx)
        {
            self.dev_runtime = true;
            self.plan_runtime(symbol, "DevContext", ctx);
            self.plan_runtime(refresh, "refresh", ctx);
        }
    }

    fn exit_function(&mut self, _func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        self.functions.pop();
    }

    fn enter_class(&mut self, class: &mut Class<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &class.id
            && is_minified(&id.name)
            && let Some(symbol) = id.symbol_id.get()
            && let Some(name) = error_class_name(class)
        {
            self.runtime_error_classes.push((symbol, name.to_string()));
        }
    }

    fn enter_string_literal(&mut self, lit: &mut StringLiteral<'a>, _ctx: &mut Ctx<'a>) {
        self.anchor(lit.value.as_str());
    }

    fn enter_template_literal(&mut self, lit: &mut TemplateLiteral<'a>, _ctx: &mut Ctx<'a>) {
        if lit.quasis.iter().all(|q| q.value.cooked.is_some()) {
            let quasis: Vec<&str> = lit
                .quasis
                .iter()
                .filter_map(|q| q.value.cooked.as_ref().map(oxc_ast::ast::Str::as_str))
                .collect();
            self.anchor(&quasis.join("${}"));
        }
    }

    /// Development runtimes read `globalThis.__turbopack_server_hmr_*` (Node.js)
    /// and the React Refresh hooks; the latter name `runModuleExecutionHooks`.
    fn enter_static_member_expression(&mut self, member: &mut StaticMemberExpression<'a>, _ctx: &mut Ctx<'a>) {
        let property = member.property.name.as_str();
        if !matches!(&member.object, Expression::Identifier(id) if matches!(id.name.as_str(), "globalThis" | "self")) {
            return;
        }
        if property.starts_with("__turbopack_server_hmr_") || property.ends_with("_CHUNK_UPDATE_LISTENERS") {
            self.dev_runtime = true;
        } else if property == "$RefreshInterceptModuleExecution$"
            && let Some(Some(symbol)) = self.functions.last()
        {
            self.dev_runtime = true;
            self.runtime_candidates.push((*symbol, "runModuleExecutionHooks"));
        }
    }

    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(registration) = program_node_chunk(program) {
            self.found.node_chunks += 1;
            self.plan_factories(registration);
        }
    }

    fn enter_object_expression(&mut self, object: &mut ObjectExpression<'a>, ctx: &mut Ctx<'a>) {
        // `{ register: globalThis.$RefreshReg$, signature: …, registerExports: registerExportsAndSetupBoundaryForReactRefresh }`
        if object_property(object, "register")
            .and_then(global_name)
            .is_some_and(|g| g == "$RefreshReg$")
            && let Some(symbol) = object_property(object, "registerExports").and_then(|e| resolve_expr(e, ctx))
        {
            self.runtime_candidates
                .push((symbol, "registerExportsAndSetupBoundaryForReactRefresh"));
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
        // `chunksToRegister.forEach(registerChunk)`
        if let Expression::StaticMemberExpression(callee) = call.callee.without_parentheses()
            && callee.property.name == "forEach"
            && let [Argument::Identifier(each)] = call.arguments.as_slice()
            && let Some(list) = resolve(each, ctx).and_then(|symbol| self.register_fns.get(&symbol).copied())
            && let Some(symbol) = resolve_expr(&callee.object, ctx)
        {
            self.plan_runtime(symbol, list, ctx);
        }
        // `_s(Component, "hash", …)` names a React Refresh signature handle.
        if let Some(handle) = resolve_expr(&call.callee, ctx)
            && self.refresh_signatures.contains(&handle)
            && let Some(Expression::Identifier(component)) = call.arguments.first().and_then(Argument::as_expression)
            && call.arguments.len() >= 2
        {
            self.refresh_signatures.remove(&handle);
            self.plan(handle, &format!("refreshSig_{}", component.name));
        }
        if self.refresh_method(&call.callee, ctx) == Some("register")
            && let [Argument::Identifier(handle), Argument::StringLiteral(id)] = call.arguments.as_slice()
            && is_identifier_name(&id.value)
            && let Some(symbol) = resolve(handle, ctx)
        {
            self.plan(symbol, &format!("refreshReg_{}", id.value));
        }

        match self.context_member(&call.callee, ctx) {
            Some("s") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first() {
                    self.plan_exports(call, list, ctx);
                }
            }
            Some("S") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first() {
                    self.record_reexports(list, ctx);
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
        if let Some((name, module)) = self.import_name(init, ctx) {
            if name.starts_with("requireContext") && !is_minified(&id.name) {
                return;
            }
            if let Some(module) = module {
                self.import_modules.insert(symbol, module);
            }
            self.plan(symbol, &name);
        } else if let Expression::CallExpression(call) = init.without_parentheses()
            && call.arguments.is_empty()
            && self.refresh_method(&call.callee, ctx) == Some("signature")
        {
            self.refresh_signatures.insert(symbol);
        } else if node_entry_runtime(init).is_some() {
            self.found.node_entries += 1;
        } else if let Some(params) = worker_param_source(init, ctx) {
            self.plan(symbol, "param");
            self.plan(params, "params");
        } else if let Some(name) = Self::singleton_name(init) {
            if is_minified(&id.name) {
                self.runtime_candidates.push((symbol, name));
            }
        } else if !self.constructors.is_empty() && self.is_prototype(init, ctx) {
            self.prototypes.insert(symbol);
            self.plan_runtime(symbol, "contextPrototype", ctx);
        }
    }

    /// `contextPrototype.i = esmImport` in the runtime chunk, the Node.js
    /// runtime's export, the runtime's registry and singletons, and the
    /// Next.js edge wrapper.
    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        if assign.operator != AssignmentOperator::Assign || self.plan_edge_entry(assign, ctx) {
            return;
        }
        if let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && let Some(name) = Self::singleton_name(&assign.right)
            && let Some(symbol) = resolve(target, ctx)
        {
            if is_minified(ctx.scoping().symbol_name(symbol)) {
                self.runtime_candidates.push((symbol, name));
            }
            return;
        }
        if self.constructors.is_empty() {
            return;
        }
        self.plan_registry_push(assign, ctx);
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
        if !is_minified(ctx.scoping().symbol_name(symbol)) {
            return;
        }
        if let Some((_, name)) = RUNTIME_MEMBERS.iter().find(|(m, _)| *m == member_name) {
            self.plan(symbol, name);
        } else if let Some(member) = RUNTIME_SPECIFIC_MEMBERS.iter().find(|m| **m == member_name) {
            self.runtime_specific.push((symbol, member));
        }
    }

    fn enter_with_statement(&mut self, _stmt: &mut WithStatement<'a>, ctx: &mut Ctx<'a>) {
        self.with_scopes.push(ctx.current_scope_id());
    }
}
