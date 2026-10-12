//! Turbopack chunk splitting for `extract-modules`, `split-chunks` and the
//! `split-sources` fallback.
//!
//! [`detect_turbopack_chunk`] reads every `TURBOPACK.push([…])` registration and
//! Node.js `module.exports = […]` chunk (see
//! [`crate::ast_deobfuscate::turbopack::shapes`]) and records, per module
//! factory, its module ids, its source range, the exports it declares through
//! `__turbopack_context__.s([…])`, the modules scope-hoisted into it
//! (`__turbopack_context__.s([…], id)`), the module ids it loads with
//! `.i(id)`, `.r(id)` or `.A(id)` and the externals it requires with `.x(…)` /
//! `.y(…)`, its `.S([…])` re-exports, the `require.context` map it exports,
//! the Next.js edge entry it registers in `self._ENTRIES`, and the
//! `// MERGED MODULE: <ident>` markers unminified scope-hoisted factories
//! carry. It also records the chunk paths of development chunk lists and
//! Node.js entries. [`TurbopackChunk::write_modules`] writes each factory as a
//! one-module chunk that registers exactly like the original.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionBody, AssignmentExpression, BindingPattern, CallExpression,
    Expression, ObjectPropertyKind, Program, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::symbol::SymbolId;
use serde::Serialize;

pub use merged::MergedModule;

mod merged;

pub use crate::ast_deobfuscate::turbopack::shapes::LoaderChunk;
use crate::ast_deobfuscate::turbopack::shapes::{
    FactoryGroup, Payload, ReexportSource, Registration, asset_path, async_loader, chunk_list, context_map, edge_entry,
    esm_bindings, factory_params, has_use_strict, module_id, module_ident, node_entry_runtime, program_node_chunk,
    reexport_groups, registration,
};

/// The Turbopack registrations of one chunk file.
#[derive(Debug, Clone, Serialize)]
pub struct TurbopackChunk {
    pub registrations: Vec<ChunkRegistration>,
    /// The `//# debugId=<uuid>` of a chunk built with debug ids
    /// (`turbopack-core/src/code_builder.rs`). Its prelude
    /// `;!function(){…e._debugIds[stack] = "<uuid>"…}();` registers the id for
    /// error reporters at load time, so it stays in every output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_id: Option<String>,
}

/// How a chunk hands its contents to the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChunkFormat {
    /// `TURBOPACK.push([script, …])`.
    Browser,
    /// `module.exports = […]`, `require`d by the Node.js runtime.
    Node,
    /// `TURBOPACK_CHUNK_LISTS.push({ script, chunks, source })` (development).
    ChunkList,
    /// `var R = require("…[turbopack]_runtime.js")(path); R.c(chunk); R.m(id)`.
    NodeEntry,
}

/// One `TURBOPACK.push([script, …])` call, Node.js chunk, chunk list or
/// Node.js entry.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkRegistration {
    pub format: ChunkFormat,
    /// Source text of the registration's callee, e.g.
    /// `(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push`,
    /// `module.exports`, or the entry's `require("./[turbopack]_runtime.js")`.
    pub callee: String,
    /// Source text of the chunk path or `document.currentScript` element (the
    /// entry's own path for a Node.js entry); `None` for a Node.js chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// `otherChunks` of the runtime chunk's registration, the `chunks` of a
    /// chunk list, or the chunks a Node.js entry loads with `R.c(…)`.
    pub other_chunks: Vec<String>,
    /// `runtimeModuleIds` of the runtime chunk's registration, or the modules a
    /// Node.js entry evaluates with `R.m(…)`.
    pub runtime_module_ids: Vec<String>,
    pub modules: Vec<ModuleFactory>,
}

/// One module factory and the module ids it is registered under.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleFactory {
    /// The chunk item's id first, then the ids of scope-hoisted modules other
    /// chunks may import through this factory.
    pub ids: Vec<String>,
    /// [`Self::ids`] as written in the chunk: a string id such as
    /// `"123456789012345678"` (above `Number.MAX_SAFE_INTEGER`) stays a string.
    #[serde(skip)]
    pub id_literals: Vec<String>,
    /// Byte range of the factory function in the chunk source.
    pub start: usize,
    pub end: usize,
    /// The factory runs in strict mode without its own directive saying so.
    pub strict: bool,
    pub exports: Vec<String>,
    pub hoisted: Vec<HoistedModule>,
    pub dependencies: Vec<String>,
    /// Requests loaded with `__turbopack_context__.x(…)` / `.y(…)`.
    pub externals: Vec<String>,
    /// `__turbopack_context__.S([…])` groups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reexports: Vec<Reexport>,
    /// The `require.context` map the module exports: `(key, module id)`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context_map: Vec<(String, String)>,
    /// The Next.js edge entry (`self._ENTRIES["middleware_<page>"]`) it registers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_entry: Option<String>,
    /// What an async loader module (`ctx.A(id)` calls it) loads and imports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_loader: Option<AsyncLoaderModule>,
    /// The path a static asset module exports (`ctx.q(path)`, `ctx.v(path)`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// `// MERGED MODULE: <ident>` markers of an unminified scope-hoisted factory.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<MergedModule>,
}

/// An async loader module: `ctx.A(loader)` loads `chunks`, then imports `module`
/// (`turbopack-ecmascript/src/async_chunk/module.rs`).
#[derive(Debug, Clone, Serialize)]
pub struct AsyncLoaderModule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    pub chunks: Vec<LoaderChunk>,
}

/// One `esmReexport` group: exports forwarded from `from` (a module id, or
/// `None` for an already imported namespace) as `(exported, imported)` pairs.
#[derive(Debug, Clone, Serialize)]
pub struct Reexport {
    pub from: Option<String>,
    pub names: Vec<(String, String)>,
}

/// A module merged into another module's factory by scope hoisting; its
/// exports are declared with `__turbopack_context__.s([…], id)`.
#[derive(Debug, Clone, Serialize)]
pub struct HoistedModule {
    pub id: String,
    pub exports: Vec<String>,
}

impl ModuleFactory {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.ids[0]
    }

    #[must_use]
    pub const fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// The Turbopack registrations in `source`, or `None` when it has none.
#[must_use]
pub fn detect_turbopack_chunk(source: &str) -> Option<TurbopackChunk> {
    crate::stack::run_on_sized_stack(source.len(), || detect_on_current_stack(source))
}

fn detect_on_current_stack(source: &str) -> Option<TurbopackChunk> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::cjs()).parse();
    if parsed.panicked {
        return None;
    }
    let program = parsed.program;
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let markers: Vec<(usize, String)> = program
        .comments
        .iter()
        .filter_map(|comment| {
            let content = comment.content_span();
            source[content.start as usize..content.end as usize]
                .strip_prefix(" MERGED MODULE: ")
                .map(|ident| (comment.span.start as usize, ident.trim_end().to_string()))
        })
        .collect();
    let mut finder = RegistrationFinder {
        source,
        scoping: &scoping,
        markers: &markers,
        registrations: Vec::new(),
        entry: None,
    };
    finder.visit_program(&program);
    (!finder.registrations.is_empty()).then(|| TurbopackChunk {
        registrations: finder.registrations,
        debug_id: debug_id(source, &program.comments),
    })
}

/// The id of a `//# debugId=<uuid>` comment.
#[must_use]
pub fn debug_id(source: &str, comments: &[oxc_ast::Comment]) -> Option<String> {
    comments.iter().rev().find_map(|comment| {
        let content = comment.content_span();
        source[content.start as usize..content.end as usize]
            .strip_prefix("# debugId=")
            .map(|id| id.trim().to_string())
    })
}

fn text(source: &str, span: oxc_span::Span) -> String {
    source[span.start as usize..span.end as usize].to_string()
}

struct RegistrationFinder<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    /// `// MERGED MODULE: <ident>` comments: start offset and ident.
    markers: &'s [(usize, String)],
    registrations: Vec<ChunkRegistration>,
    /// The Node.js entry's runtime binding `R` and its registration's index.
    entry: Option<(SymbolId, usize)>,
}

impl RegistrationFinder<'_> {
    fn module_factory(&self, group: &FactoryGroup<'_, '_>) -> ModuleFactory {
        let factory_span = group.factory.span();
        let (scope, has_directive, body) = match group.factory.without_parentheses() {
            Expression::FunctionExpression(func) => (
                func.scope_id.get(),
                func.body.as_ref().is_some_and(|body| has_use_strict(body)),
                func.body.as_deref(),
            ),
            Expression::ArrowFunctionExpression(arrow) => (
                arrow.scope_id.get(),
                arrow.has_use_strict_directive(),
                match &arrow.body {
                    ArrowFunctionBody::FunctionBody(body) => Some(&**body),
                    _ => None,
                },
            ),
            _ => (None, false, None),
        };
        let strict = !has_directive
            && (group.strict_wrapper || scope.is_some_and(|scope| self.scoping.scope_flags(scope).is_strict_mode()));
        let mut scan = FactoryScan {
            scoping: self.scoping,
            context: factory_params(group.factory)
                .and_then(|params| params.items.first())
                .and_then(|param| match &param.pattern {
                    BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
                    _ => None,
                }),
            exports: Vec::new(),
            hoisted: Vec::new(),
            dependencies: Vec::new(),
            externals: Vec::new(),
            reexports: Vec::new(),
            context_map: Vec::new(),
            edge_entry: None,
            async_loader: None,
            asset: None,
        };
        if scan.context.is_some() {
            scan.visit_expression(group.factory);
        }
        let merged = body
            .filter(|body| {
                let range = body.span.start as usize..body.span.end as usize;
                self.markers.iter().any(|(start, _)| range.contains(start))
            })
            .map_or_else(Vec::new, |body| {
                merged::merged_modules(
                    &body.statements,
                    body.span.start as usize,
                    self.markers,
                    self.scoping,
                    scan.context,
                )
            });
        ModuleFactory {
            ids: group.ids.clone(),
            id_literals: group.id_spans.iter().map(|&span| text(self.source, span)).collect(),
            start: factory_span.start as usize,
            end: factory_span.end as usize,
            strict,
            exports: scan.exports,
            hoisted: scan.hoisted,
            dependencies: scan.dependencies,
            externals: scan.externals,
            reexports: scan.reexports,
            context_map: scan.context_map,
            edge_entry: scan.edge_entry,
            async_loader: scan.async_loader,
            asset: scan.asset,
            merged,
        }
    }

    fn record(&mut self, format: ChunkFormat, callee: String, found: Registration<'_, '_>) {
        let mut record = ChunkRegistration {
            format,
            callee,
            script: found.script.map(|script| text(self.source, script.span())),
            other_chunks: Vec::new(),
            runtime_module_ids: Vec::new(),
            modules: Vec::new(),
        };
        match found.payload {
            Payload::Runtime(params) => {
                for prop in &params.properties {
                    let ObjectPropertyKind::ObjectProperty(prop) = prop else {
                        continue;
                    };
                    if prop.key.is_specific_static_name("otherChunks") {
                        record.other_chunks = string_list(&prop.value);
                    } else if prop.key.is_specific_static_name("runtimeModuleIds") {
                        record.runtime_module_ids = string_list(&prop.value);
                    }
                }
            }
            Payload::Factories(groups) => {
                record.modules = groups.iter().map(|group| self.module_factory(group)).collect();
            }
        }
        self.registrations.push(record);
    }

    /// `R.c(chunk)` / `R.m(id)` on the Node.js entry's runtime binding.
    fn entry_call(&mut self, call: &CallExpression<'_>) -> bool {
        let Some((runtime, index)) = self.entry else {
            return false;
        };
        let Expression::StaticMemberExpression(member) = call.callee.without_parentheses() else {
            return false;
        };
        let Expression::Identifier(object) = &member.object else {
            return false;
        };
        if self.scoping.get_reference(object.reference_id()).symbol_id() != Some(runtime) {
            return false;
        }
        let Some(arg) = first_module_id(&call.arguments, 0) else {
            return false;
        };
        let record = &mut self.registrations[index];
        let list = match member.property.name.as_str() {
            "c" => &mut record.other_chunks,
            "m" => &mut record.runtime_module_ids,
            _ => return false,
        };
        if !list.contains(&arg) {
            list.push(arg);
        }
        true
    }
}

/// The strings or numbers of an array literal; a chunk's `{ path, … }` object
/// gives its `path`.
fn string_list(expr: &Expression<'_>) -> Vec<String> {
    let Expression::ArrayExpression(list) = expr.without_parentheses() else {
        return Vec::new();
    };
    list.elements
        .iter()
        .filter_map(ArrayExpressionElement::as_expression)
        .filter_map(|element| match element.without_parentheses() {
            Expression::ObjectExpression(chunk) => chunk.properties.iter().find_map(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name("path") => module_id(&p.value),
                _ => None,
            }),
            element => module_id(element),
        })
        .collect()
}

impl<'a> Visit<'a> for RegistrationFinder<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        if let Some(found) = program_node_chunk(program) {
            self.record(ChunkFormat::Node, "module.exports".to_string(), found);
        }
        walk::walk_program(self, program);
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &decl.id
            && let Some(symbol) = id.symbol_id.get()
            && let Some(init) = &decl.init
            && node_entry_runtime(init).is_some()
            && let Expression::CallExpression(init) = init.without_parentheses()
        {
            self.entry = Some((symbol, self.registrations.len()));
            self.registrations.push(ChunkRegistration {
                format: ChunkFormat::NodeEntry,
                callee: text(self.source, init.callee.span()),
                script: init.arguments.first().map(|path| text(self.source, path.span())),
                other_chunks: Vec::new(),
                runtime_module_ids: Vec::new(),
                modules: Vec::new(),
            });
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Some(found) = registration(call) {
            self.record(ChunkFormat::Browser, text(self.source, call.callee.span()), found);
            return;
        }
        if let Some(list) = chunk_list(call) {
            self.registrations.push(ChunkRegistration {
                format: ChunkFormat::ChunkList,
                callee: text(self.source, call.callee.span()),
                script: Some(text(self.source, list.script.span())),
                other_chunks: string_list(list.chunks),
                runtime_module_ids: Vec::new(),
                modules: Vec::new(),
            });
            return;
        }
        if self.entry_call(call) {
            return;
        }
        walk::walk_call_expression(self, call);
    }
}

/// Reads the `__turbopack_context__` calls of one factory.
struct FactoryScan<'s> {
    scoping: &'s Scoping,
    context: Option<SymbolId>,
    exports: Vec<String>,
    hoisted: Vec<HoistedModule>,
    dependencies: Vec<String>,
    externals: Vec<String>,
    reexports: Vec<Reexport>,
    context_map: Vec<(String, String)>,
    edge_entry: Option<String>,
    async_loader: Option<AsyncLoaderModule>,
    asset: Option<String>,
}

impl FactoryScan<'_> {
    fn depend_on(&mut self, id: String) {
        if !self.dependencies.contains(&id) {
            self.dependencies.push(id);
        }
    }

    fn context_member<'b>(&self, callee: &'b Expression<'_>) -> Option<&'b str> {
        let Expression::StaticMemberExpression(member) = callee.without_parentheses() else {
            return None;
        };
        let Expression::Identifier(object) = member.object.without_parentheses() else {
            return None;
        };
        let symbol = self.scoping.get_reference(object.reference_id()).symbol_id();
        (symbol.is_some() && symbol == self.context).then_some(member.property.name.as_str())
    }
}

fn first_module_id(args: &[Argument<'_>], index: usize) -> Option<String> {
    args.get(index)?.as_expression().and_then(module_id)
}

impl<'a> Visit<'a> for FactoryScan<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        match self.context_member(&call.callee) {
            Some("s") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first() {
                    let names: Vec<String> = esm_bindings(list)
                        .into_iter()
                        .map(|(name, _)| name.to_string())
                        .collect();
                    match first_module_id(&call.arguments, 1) {
                        Some(id) => self.hoisted.push(HoistedModule { id, exports: names }),
                        None => self.exports.extend(names),
                    }
                }
            }
            Some("S") => {
                if let Some(Argument::ArrayExpression(list)) = call.arguments.first()
                    && let Some(groups) = reexport_groups(list)
                {
                    let mut names = Vec::new();
                    for group in groups {
                        let from = match group.source {
                            ReexportSource::Module(id) => {
                                self.depend_on(id.clone());
                                Some(id)
                            }
                            ReexportSource::Namespace(_) => None,
                        };
                        names.extend(group.pairs.iter().map(|(exported, _)| exported.clone()));
                        self.reexports.push(Reexport {
                            from,
                            names: group.pairs,
                        });
                    }
                    match first_module_id(&call.arguments, 1) {
                        Some(id) => self.hoisted.push(HoistedModule { id, exports: names }),
                        None => self.exports.extend(names),
                    }
                }
            }
            Some("i" | "r" | "A") => {
                if let Some(id) = first_module_id(&call.arguments, 0) {
                    self.depend_on(id);
                }
            }
            Some("v") if !matches!(call.arguments.first(), Some(Argument::ObjectExpression(_))) => {
                let value = call.arguments.first().and_then(Argument::as_expression);
                if let Some(loader) = value.and_then(async_loader) {
                    self.async_loader = Some(AsyncLoaderModule {
                        module: loader.module,
                        chunks: loader.chunks,
                    });
                } else if let Some(path) = value.and_then(asset_path) {
                    self.asset = Some(path.to_string());
                }
            }
            Some("q") => {
                if let [Argument::StringLiteral(path)] = call.arguments.as_slice() {
                    self.asset = Some(path.value.to_string());
                }
            }
            Some("v" | "f") => {
                if let Some(Argument::ObjectExpression(map)) = call.arguments.first()
                    && let Some(entries) = context_map(map)
                {
                    for (_, id) in &entries {
                        self.depend_on(id.clone());
                    }
                    self.context_map.extend(entries);
                }
            }
            Some("x" | "y") => {
                if let Some(Argument::StringLiteral(request)) = call.arguments.first()
                    && !self.externals.iter().any(|e| e == request.value.as_str())
                {
                    self.externals.push(request.value.to_string());
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        if let Some((name, _)) = edge_entry(assign) {
            self.edge_entry = Some(name.into_owned());
        }
        walk::walk_assignment_expression(self, assign);
    }
}

impl TurbopackChunk {
    pub fn modules(&self) -> impl Iterator<Item = &ModuleFactory> {
        self.registrations.iter().flat_map(|r| r.modules.iter())
    }

    #[must_use]
    pub fn module_count(&self) -> usize {
        self.modules().count()
    }

    #[must_use]
    pub fn hoisted_count(&self) -> usize {
        self.modules().map(|m| m.hoisted.len()).sum()
    }

    /// One `(file name, contents)` per module factory, then one comment-only
    /// stub per scope-hoisted module under `hoisted/` and the source part of
    /// each `MERGED MODULE` marker under `merged/`. Each module file is a
    /// chunk registering just that factory, under all of its ids, exactly as
    /// the original chunk did.
    #[must_use]
    pub fn module_files(&self, source: &str) -> Vec<(String, String)> {
        let mut names = BTreeMap::new();
        let mut out = Vec::new();
        let mut stubs = Vec::new();
        for registration in &self.registrations {
            for module in &registration.modules {
                let file = unique_name(&mut names, format!("module_{}.js", module_ident(module.id())));
                out.push((file.clone(), module_file(source, registration, module)));
                for hoisted in &module.hoisted {
                    let stub = unique_name(&mut names, format!("hoisted/module_{}.js", module_ident(&hoisted.id)));
                    stubs.push((stub, hoisted_stub(hoisted, module, &file)));
                }
                for (index, merged) in module.merged.iter().enumerate() {
                    let part = unique_name(&mut names, format!("merged/module_{}.js", module_ident(&merged.ident)));
                    stubs.push((part, merged_part(source, module, index, &file)));
                }
            }
        }
        out.extend(stubs);
        out
    }

    /// Write [`Self::module_files`] under `dir` and return the relative paths.
    ///
    /// # Errors
    /// Returns an error when a directory or file cannot be written.
    pub fn write_modules(&self, source: &str, dir: &Path) -> io::Result<Vec<String>> {
        let files = self.module_files(source);
        for (name, contents) in &files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, contents)?;
        }
        Ok(files.into_iter().map(|(name, _)| name).collect())
    }

    /// The module dependency graph in DOT format.
    #[must_use]
    pub fn dependency_graph(&self) -> String {
        let mut graph = String::from("digraph turbopack_modules {\n  rankdir=LR;\n  node [shape=box];\n\n");
        let loaded: BTreeMap<&str, &str> = self
            .modules()
            .filter_map(|m| Some((m.id(), m.async_loader.as_ref()?.module.as_deref()?)))
            .collect();
        for module in self.modules() {
            for dep in &module.dependencies {
                let _ = writeln!(graph, "  \"{}\" -> \"{}\";", dot_escape(module.id()), dot_escape(dep));
                // `ctx.A(loader)` imports the loader's module once its chunks are loaded.
                if let Some(target) = loaded.get(dep.as_str()) {
                    let _ = writeln!(
                        graph,
                        "  \"{}\" -> \"{}\" [style=dashed, label=\"import()\"];",
                        dot_escape(module.id()),
                        dot_escape(target)
                    );
                }
            }
        }
        graph.push_str("}\n");
        graph
    }
}

fn dot_escape(id: &str) -> String {
    id.replace('\\', "\\\\").replace('"', "\\\"")
}

fn id_literal(id: &str) -> String {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        id.to_string()
    } else {
        serde_json::to_string(id).unwrap_or_default()
    }
}

fn module_file(source: &str, registration: &ChunkRegistration, module: &ModuleFactory) -> String {
    let mut out = String::new();
    let _ = write!(out, "// Turbopack module {}", module.id());
    if module.ids.len() > 1 {
        let _ = write!(out, " (also registered as {})", module.ids[1..].join(", "));
    }
    out.push('\n');
    if !module.exports.is_empty() {
        let _ = writeln!(out, "// exports: {}", module.exports.join(", "));
    }
    for hoisted in &module.hoisted {
        let _ = writeln!(
            out,
            "// scope-hoisted module {}: {}",
            hoisted.id,
            hoisted.exports.join(", ")
        );
    }
    if !module.dependencies.is_empty() {
        let _ = writeln!(out, "// imports: {}", module.dependencies.join(", "));
    }
    if !module.externals.is_empty() {
        let _ = writeln!(out, "// externals: {}", module.externals.join(", "));
    }
    for reexport in module.reexports.iter().filter(|r| !r.names.is_empty()) {
        let names: Vec<String> = reexport
            .names
            .iter()
            .map(|(exported, imported)| {
                if exported == imported {
                    exported.clone()
                } else {
                    format!("{imported} as {exported}")
                }
            })
            .collect();
        let from = reexport.from.as_deref().unwrap_or("an imported namespace");
        let _ = writeln!(out, "// re-exports from {from}: {}", names.join(", "));
    }
    if !module.context_map.is_empty() {
        let entries: Vec<String> = module
            .context_map
            .iter()
            .map(|(key, id)| format!("{key} -> {id}"))
            .collect();
        let _ = writeln!(out, "// require.context: {}", entries.join(", "));
    }
    if let Some(entry) = &module.edge_entry {
        let _ = writeln!(out, "// edge entry: self._ENTRIES[{}]", id_literal(entry));
    }
    if let Some(loader) = &module.async_loader {
        let chunks: Vec<&str> = loader.chunks.iter().map(|chunk| chunk.path.as_str()).collect();
        let _ = writeln!(
            out,
            "// async loader: loads [{}], then imports {}",
            chunks.join(", "),
            loader.module.as_deref().unwrap_or("nothing")
        );
    }
    if let Some(asset) = &module.asset {
        let _ = writeln!(out, "// asset: {asset}");
    }
    for merged in &module.merged {
        let _ = writeln!(out, "// merged module: {}", merged.ident);
    }
    if module.strict {
        out.push_str("\"use strict\";\n");
    }
    let ids = &module.id_literals;
    let factory = &source[module.range()];
    match &registration.script {
        Some(script) => {
            let _ = writeln!(
                out,
                "{}([{script}, {}, {factory}]);",
                registration.callee,
                ids.join(", ")
            );
        }
        None => {
            let _ = writeln!(out, "{} = [{}, {factory}];", registration.callee, ids.join(", "));
        }
    }
    out
}

fn hoisted_stub(hoisted: &HoistedModule, module: &ModuleFactory, file: &str) -> String {
    format!(
        "// Turbopack module {} is scope-hoisted into module {} ({file}),\n\
         // which declares its exports with __turbopack_context__.s([…], {}).\n\
         // exports: {}\n",
        hoisted.id,
        module.id(),
        id_literal(&hoisted.id),
        hoisted.exports.join(", ")
    )
}

fn merged_part(source: &str, module: &ModuleFactory, index: usize, file: &str) -> String {
    let merged = &module.merged[index];
    let mut out = format!(
        "// Turbopack module {} is scope-hoisted into module {} ({file}).\n",
        merged.ident,
        module.id()
    );
    if let Some(parent) = merged.parent {
        let _ = writeln!(out, "// Inlined into {}.", module.merged[parent].ident);
    }
    let mut pieces: Vec<(usize, String)> = merged
        .own
        .iter()
        .map(|&(start, end)| (start, source[start..end].to_string()))
        .collect();
    pieces.extend(
        module
            .merged
            .iter()
            .filter(|child| child.parent == Some(index))
            .map(|child| (child.start, format!("// (inlined here: {})", child.ident))),
    );
    pieces.sort_by_key(|(start, _)| *start);
    for (_, piece) in pieces {
        out.push_str(piece.trim_end());
        out.push('\n');
    }
    out
}

fn unique_name(names: &mut BTreeMap<String, usize>, name: String) -> String {
    let count = names.entry(name.clone()).or_default();
    *count += 1;
    if *count == 1 {
        return name;
    }
    let suffix = *count;
    name.rsplit_once('.').map_or_else(
        || format!("{name}_{suffix}"),
        |(stem, ext)| format!("{stem}_{suffix}.{ext}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK: &str = r#";!function(){var e=1}();
(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,
(()=>{"use strict";return[7,e=>{e.v(1)}]})(),
983,e=>{"use strict";var t=e.i(913777),r=e.A(5);function n(){}e.s(["Spinner",0,n,"value",()=>t,v=>{t=v}]);e.s(["inner",()=>n],784018);function f(e){e.r(1)}},
11,"[project]/a.js",(e,t,r)=>{t.exports=e.r(913777)}
]);
//# sourceMappingURL=c.js.map"#;

    fn chunk() -> TurbopackChunk {
        detect_turbopack_chunk(CHUNK).expect("turbopack chunk")
    }

    #[test]
    fn reads_factories_ids_exports_and_hoisted_modules() {
        let chunk = chunk();
        assert_eq!(chunk.registrations.len(), 1);
        let modules: Vec<_> = chunk.modules().collect();
        assert_eq!(modules.len(), 3);
        assert_eq!(modules[0].ids, ["7"]);
        assert!(modules[0].strict, "strict IIFE factory");
        assert_eq!(modules[1].ids, ["983"]);
        assert!(!modules[1].strict, "has its own directive");
        assert_eq!(modules[1].exports, ["Spinner", "value"]);
        assert_eq!(modules[1].hoisted.len(), 1);
        assert_eq!(modules[1].hoisted[0].id, "784018");
        assert_eq!(modules[1].hoisted[0].exports, ["inner"]);
        assert_eq!(
            modules[1].dependencies,
            ["913777", "5"],
            "shadowed `e.r(1)` is not a dependency"
        );
        assert_eq!(modules[2].ids, ["11", "[project]/a.js"]);
        assert!(CHUNK[modules[2].range()].starts_with("(e,t,r)=>"));
    }

    #[test]
    fn module_files_reregister_one_factory_each() {
        let files = chunk().module_files(CHUNK);
        let names: Vec<_> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "module_7.js",
                "module_983.js",
                "module_11.js",
                "hoisted/module_784018.js"
            ]
        );
        let (_, strict) = &files[0];
        assert!(strict.contains("\"use strict\";\n(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([\"object\"==typeof document?document.currentScript:void 0, 7, e=>{e.v(1)}]);"), "{strict}");
        let (_, aliased) = &files[2];
        assert!(
            aliased.starts_with("// Turbopack module 11 (also registered as [project]/a.js)\n"),
            "{aliased}"
        );
        assert!(aliased.contains(", 11, \"[project]/a.js\", (e,t,r)=>"), "{aliased}");
        let (_, stub) = &files[3];
        assert!(stub.contains("scope-hoisted into module 983 (module_983.js)"), "{stub}");
        for (name, contents) in &files {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, contents, SourceType::cjs()).parse();
            assert!(parsed.diagnostics.is_empty(), "{name} must parse: {contents}");
            assert!(
                name.starts_with("hoisted/") || detect_turbopack_chunk(contents).is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn reads_runtime_registration() {
        let source = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([document.currentScript,{otherChunks:["static/chunks/a.js"],runtimeModuleIds:[570711]}]);"#;
        let chunk = detect_turbopack_chunk(source).expect("runtime registration");
        assert_eq!(chunk.module_count(), 0);
        assert_eq!(chunk.registrations[0].other_chunks, ["static/chunks/a.js"]);
        assert_eq!(chunk.registrations[0].runtime_module_ids, ["570711"]);
    }

    #[test]
    fn node_chunk_modules_reexport_one_factory_each() {
        let source = r#"(()=>{"use strict";module.exports=["[project]/src/a.ts [app-ssr] (ecmascript)",t=>{t.s(["f",0,1]),t.x("react",()=>require("react"))},12,13,(t,e,o)=>{e.exports=t.r(12)}]})();"#;
        let chunk = detect_turbopack_chunk(source).expect("node chunk");
        assert_eq!(chunk.registrations[0].format, ChunkFormat::Node);
        assert!(chunk.registrations[0].script.is_none());
        let modules: Vec<_> = chunk.modules().collect();
        assert_eq!(modules[0].ids, ["[project]/src/a.ts [app-ssr] (ecmascript)"]);
        assert!(modules[0].strict, "all-strict wrapper");
        assert_eq!(modules[0].externals, ["react"]);
        assert_eq!(modules[1].dependencies, ["12"]);
        let files = chunk.module_files(source);
        assert_eq!(files[0].0, "module_src_a_ts.js");
        assert!(
            files[0]
                .1
                .starts_with("// Turbopack module [project]/src/a.ts [app-ssr] (ecmascript)\n"),
            "{}",
            files[0].1
        );
        assert!(files[0].1.contains("// externals: react\n"), "{}", files[0].1);
        assert!(
            files[1]
                .1
                .ends_with("module.exports = [12, 13, (t,e,o)=>{e.exports=t.r(12)}];\n"),
            "{}",
            files[1].1
        );
        for (name, contents) in &files {
            let module = detect_turbopack_chunk(contents).unwrap_or_else(|| panic!("{name}: {contents}"));
            assert_eq!(module.registrations[0].format, ChunkFormat::Node);
        }
    }

    #[test]
    fn reads_chunk_lists_entries_and_bootstrap_objects() {
        let list = r#"(globalThis["TURBOPACK_CHUNK_LISTS"] || (globalThis["TURBOPACK_CHUNK_LISTS"] = [])).push({script: document.currentScript, chunks: ["static/chunks/a.js", "static/chunks/b.js"], source: "entry"});"#;
        let chunk = detect_turbopack_chunk(list).expect("chunk list");
        assert_eq!(chunk.registrations[0].format, ChunkFormat::ChunkList);
        assert_eq!(
            chunk.registrations[0].other_chunks,
            ["static/chunks/a.js", "static/chunks/b.js"]
        );

        let entry = "var R=require(\"../chunks/ssr/[turbopack]_runtime.js\")(\"server/app/page.js\")\nR.c(\"server/chunks/ssr/a.js\")\nR.c(\"server/chunks/ssr/b.js\")\nR.m(12)\nmodule.exports=R.m(34).exports";
        let chunk = detect_turbopack_chunk(entry).expect("node entry");
        let registration = &chunk.registrations[0];
        assert_eq!(registration.format, ChunkFormat::NodeEntry);
        assert_eq!(registration.callee, "require(\"../chunks/ssr/[turbopack]_runtime.js\")");
        assert_eq!(registration.script.as_deref(), Some("\"server/app/page.js\""));
        assert_eq!(
            registration.other_chunks,
            ["server/chunks/ssr/a.js", "server/chunks/ssr/b.js"]
        );
        assert_eq!(registration.runtime_module_ids, ["12", "34"]);

        let bootstrap = r#"(globalThis["TURBOPACK"] || (globalThis["TURBOPACK"] = [])).push({"otherChunks":["a.js",{"path":"b.js","included":[1],"excluded":[],"moduleChunks":[]}],"runtimeModuleIds":[7]});"#;
        let chunk = detect_turbopack_chunk(bootstrap).expect("bootstrap");
        assert!(chunk.registrations[0].script.is_none());
        assert_eq!(chunk.registrations[0].other_chunks, ["a.js", "b.js"]);
        assert_eq!(chunk.registrations[0].runtime_module_ids, ["7"]);
    }

    #[test]
    fn reads_reexports_context_maps_edge_entries_and_nested_lists() {
        let source = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["chunks/a.js",[52482,e=>{e.S([96154,0,13477,"workAsyncStorage,f"])},16448,e=>{var n=e.i(26088);e.S([n,"getCacheSignal","a"],16449)}],52812,function(e){e.v({"./moon.js":{id:()=>3456,module:()=>e.r(3456)},"./star.js":{id:()=>28417,module:()=>e.r(28417)}})},28517,(e,t,n)=>{self._ENTRIES||={};let s=Promise.resolve().then(()=>e.i(55658));self._ENTRIES["middleware_app/page"]=new Proxy(s,{get(e,t){return e}})}]);"#;
        let chunk = detect_turbopack_chunk(source).expect("chunk with a nested list");
        let modules: Vec<_> = chunk.modules().collect();
        assert_eq!(modules.len(), 4);
        assert_eq!(modules[0].ids, ["52482"]);
        assert_eq!(modules[0].exports, ["workAsyncStorage"]);
        assert_eq!(modules[0].dependencies, ["96154", "13477"]);
        assert_eq!(modules[0].reexports.len(), 2);
        assert_eq!(modules[0].reexports[0].names.len(), 0);
        assert_eq!(
            modules[0].reexports[1].names,
            [("workAsyncStorage".to_string(), "f".to_string())]
        );
        assert_eq!(modules[1].exports.len(), 0);
        assert_eq!(modules[1].hoisted[0].id, "16449");
        assert_eq!(modules[1].hoisted[0].exports, ["getCacheSignal"]);
        assert_eq!(modules[1].reexports[0].from, None);
        assert_eq!(
            modules[2].context_map,
            [
                ("./moon.js".to_string(), "3456".to_string()),
                ("./star.js".to_string(), "28417".to_string())
            ]
        );
        assert_eq!(modules[2].dependencies, ["3456", "28417"]);
        assert_eq!(modules[3].edge_entry.as_deref(), Some("middleware_app/page"));
        let files = chunk.module_files(source);
        let reexporter = &files[0].1;
        assert!(
            reexporter.contains("// imports: 96154, 13477\n// re-exports from 13477: f as workAsyncStorage\n"),
            "{reexporter}"
        );
        assert!(
            files[2]
                .1
                .contains("// require.context: ./moon.js -> 3456, ./star.js -> 28417\n"),
            "{}",
            files[2].1
        );
        assert!(
            files[3]
                .1
                .contains("// edge entry: self._ENTRIES[\"middleware_app/page\"]\n"),
            "{}",
            files[3].1
        );
    }

    #[test]
    fn splits_merged_module_markers() {
        let source = "module.exports = [\n99665, ((__turbopack_context__) => {\n\"use strict\";\n\n// MERGED MODULE: [project]/src/app/api/hello/route.ts [app-route] (ecmascript)\n;\n// MERGED MODULE: [project]/src/lib/shared.ts [app-route] (ecmascript)\n;\nfunction greeting(name) {\n    return name;\n}\nconst VERSION = 1;\n;\nasync function GET() {}\n__turbopack_context__.s([\n    \"GET\",\n    0,\n    GET\n], 99665);\n}),\n];\n";
        let chunk = detect_turbopack_chunk(source).expect("node chunk");
        let module = chunk.modules().next().expect("one factory");
        let idents: Vec<_> = module.merged.iter().map(|m| m.ident.as_str()).collect();
        assert_eq!(
            idents,
            [
                "[project]/src/app/api/hello/route.ts [app-route] (ecmascript)",
                "[project]/src/lib/shared.ts [app-route] (ecmascript)"
            ]
        );
        let (route, shared) = (&module.merged[0], &module.merged[1]);
        assert_eq!(shared.parent, Some(0));
        assert!(source[route.start..route.end].ends_with("], 99665);"));
        assert!(
            source[shared.start..shared.end].ends_with("const VERSION = 1;"),
            "the route's removed import `;` starts its body"
        );
        let files = chunk.module_files(source);
        let names: Vec<_> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "module_99665.js",
                "hoisted/module_99665.js",
                "merged/module_src_app_api_hello_route_ts.js",
                "merged/module_src_lib_shared_ts.js"
            ]
        );
        assert!(
            files[0]
                .1
                .contains("// merged module: [project]/src/lib/shared.ts [app-route] (ecmascript)\n")
        );
        let shared = &files[3].1;
        assert!(
            shared.contains("is scope-hoisted into module 99665 (module_99665.js)"),
            "{shared}"
        );
        assert!(shared.contains("function greeting(name)"), "{shared}");
        assert!(!shared.contains("GET"), "{shared}");
        let route = &files[2].1;
        assert!(route.contains("async function GET() {}"), "{route}");
        assert!(
            route.contains("// (inlined here: [project]/src/lib/shared.ts"),
            "{route}"
        );
        assert!(!route.contains("function greeting"), "{route}");
    }

    /// Shapes from the unminified Next.js 16.4 build: an import variable after
    /// an inlined module returns to the importer's hoisted part, a statement
    /// reading an importer's import variable is the importer's, and `s(…, id)`
    /// ends a module.
    #[test]
    fn merged_module_extents_follow_hoisting() {
        let source = r#"module.exports = [
1, 2, ((__turbopack_context__) => {
"use strict";
// MERGED MODULE: [project]/a.js [ssr] (ecmascript)
;
var __TURBOPACK__imported__module__7__ = __turbopack_context__.i(7);
// MERGED MODULE: [project]/b.js [ssr] (ecmascript)
;
function b() {}
var __TURBOPACK__imported__module__8__ = __turbopack_context__.i(8);
// MERGED MODULE: [project]/c.js [ssr] (ecmascript)
;
var __TURBOPACK__imported__module__7__1 = __TURBOPACK__imported__module__7__;
;
const c = __TURBOPACK__imported__module__7__1.x;
__turbopack_context__.s(["c", 0, c], 2);
;
;
;
'use client';
const a = __TURBOPACK__imported__module__8__.y + b() + c;
__turbopack_context__.s(["a", 0, a], 1);
}),
];
"#;
        let chunk = detect_turbopack_chunk(source).expect("node chunk");
        let merged = &chunk.modules().next().expect("one factory").merged;
        let text = |m: &MergedModule| source[m.start..m.end].to_string();
        let own = |m: &MergedModule| -> Vec<&str> { m.own.iter().map(|&(s, e)| source[s..e].trim()).collect() };
        let (a, b, c) = (&merged[0], &merged[1], &merged[2]);
        assert_eq!((b.parent, c.parent), (Some(0), Some(0)));
        assert!(text(b).ends_with("function b() {}"), "{}", text(b));
        assert!(text(c).ends_with("], 2);"), "{}", text(c));
        assert!(text(a).ends_with("], 1);"), "{}", text(a));
        let a_own = own(a).join("\n");
        assert!(a_own.contains("__turbopack_context__.i(8)"), "{a_own}");
        assert!(a_own.contains("'use client';\nconst a ="), "{a_own}");
        assert!(!a_own.contains("function b"), "{a_own}");
    }

    #[test]
    fn reads_async_loaders_assets_debug_ids_and_string_ids() {
        let source = r#";!function(){try{var e=globalThis,n=(new e.Error).stack;n&&((e._debugIds||(e._debugIds={}))[n]="8b06e878-84da-932e-79ca-1f7f27f8946b")}catch(e){}}();
module.exports=[79095,a=>{a.v(b=>Promise.all(["server/chunks/ssr/heavy.js"].map(b=>a.l(b))).then(()=>b(34526)))},18400,function(e){e.q("server/chunks/add.wasm")},47678,a=>{a.v("/_next/static/media/w.ts"+(globalThis.NEXT_CLIENT_ASSET_SUFFIX||""))},"123456789012345678",e=>{e.A(79095)},5,e=>{e.v(t=>Promise.all([["static/chunks/m.js",["static/chunks/a.js","static/chunks/b.js"],[10,20]],{path:"static/chunks/c.js",included:[]}].map(t=>e.l(t))).then(()=>{}))}];
//# debugId=8b06e878-84da-932e-79ca-1f7f27f8946b"#;
        let chunk = detect_turbopack_chunk(source).expect("node chunk");
        assert_eq!(chunk.debug_id.as_deref(), Some("8b06e878-84da-932e-79ca-1f7f27f8946b"));
        let modules: Vec<_> = chunk.modules().collect();
        let loader = modules[0].async_loader.as_ref().expect("async loader");
        assert_eq!(loader.module.as_deref(), Some("34526"));
        assert_eq!(loader.chunks[0].path, "server/chunks/ssr/heavy.js");
        assert_eq!(modules[1].asset.as_deref(), Some("server/chunks/add.wasm"));
        assert_eq!(modules[2].asset.as_deref(), Some("/_next/static/media/w.ts"));
        let merged = &modules[4].async_loader.as_ref().expect("loader without module").chunks;
        assert_eq!(modules[4].async_loader.as_ref().unwrap().module, None);
        assert_eq!(merged[0].path, "static/chunks/m.js");
        assert_eq!(merged[0].components, ["static/chunks/a.js", "static/chunks/b.js"]);
        assert_eq!(merged[1].path, "static/chunks/c.js");
        let graph = chunk.dependency_graph();
        assert!(
            graph.contains(r#""123456789012345678" -> "34526" [style=dashed, label="import()"];"#),
            "{graph}"
        );
        let files = chunk.module_files(source);
        assert!(
            files[0]
                .1
                .contains("// async loader: loads [server/chunks/ssr/heavy.js], then imports 34526\n"),
            "{}",
            files[0].1
        );
        assert!(
            files[1].1.contains("// asset: server/chunks/add.wasm\n"),
            "{}",
            files[1].1
        );
        assert!(
            files[3]
                .1
                .contains(r#"module.exports = ["123456789012345678", e=>{e.A(79095)}];"#),
            "a string id above MAX_SAFE_INTEGER stays a string: {}",
            files[3].1
        );
    }

    #[test]
    fn ignores_other_bundles() {
        assert!(detect_turbopack_chunk("(self.webpackChunk=self.webpackChunk||[]).push([[1],{1(e){}}]);").is_none());
        assert!(detect_turbopack_chunk("var a = [1, function(){}]; a.push([2, e=>{}]);").is_none());
    }
}
