//! Turbopack chunk splitting for `extract-modules`, `split-chunks` and the
//! `split-sources` fallback.
//!
//! [`detect_turbopack_chunk`] reads every `TURBOPACK.push([…])` registration
//! (see [`crate::ast_deobfuscate::turbopack::shapes`]) and records, per module
//! factory, its module ids, its source range, the exports it declares through
//! `__turbopack_context__.s([…])`, the modules scope-hoisted into it
//! (`__turbopack_context__.s([…], id)`) and the module ids it loads with
//! `.i(id)`, `.r(id)` or `.A(id)`. [`TurbopackChunk::write_modules`] writes each
//! factory as a one-module chunk that registers exactly like the original.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, ArrayExpressionElement, BindingPattern, CallExpression, Expression, ObjectPropertyKind};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::symbol::SymbolId;
use serde::Serialize;

use crate::ast_deobfuscate::turbopack::shapes::{
    FactoryGroup, Payload, esm_bindings, factory_params, has_use_strict, module_id, registration,
};

/// The Turbopack registrations of one chunk file.
#[derive(Debug, Clone, Serialize)]
pub struct TurbopackChunk {
    pub registrations: Vec<ChunkRegistration>,
}

/// One `TURBOPACK.push([script, …])` call.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkRegistration {
    /// Source text of the registration's callee, e.g.
    /// `(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push`.
    pub callee: String,
    /// Source text of the chunk path or `document.currentScript` element.
    pub script: String,
    /// `otherChunks` of the runtime chunk's registration.
    pub other_chunks: Vec<String>,
    /// `runtimeModuleIds` of the runtime chunk's registration.
    pub runtime_module_ids: Vec<String>,
    pub modules: Vec<ModuleFactory>,
}

/// One module factory and the module ids it is registered under.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleFactory {
    /// The chunk item's id first, then the ids of scope-hoisted modules other
    /// chunks may import through this factory.
    pub ids: Vec<String>,
    /// Byte range of the factory function in the chunk source.
    pub start: usize,
    pub end: usize,
    /// The factory runs in strict mode without its own directive saying so.
    pub strict: bool,
    pub exports: Vec<String>,
    pub hoisted: Vec<HoistedModule>,
    pub dependencies: Vec<String>,
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
    let mut finder = RegistrationFinder {
        source,
        scoping: &scoping,
        registrations: Vec::new(),
    };
    finder.visit_program(&program);
    (!finder.registrations.is_empty()).then_some(TurbopackChunk {
        registrations: finder.registrations,
    })
}

fn text(source: &str, span: oxc_span::Span) -> String {
    source[span.start as usize..span.end as usize].to_string()
}

struct RegistrationFinder<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    registrations: Vec<ChunkRegistration>,
}

impl RegistrationFinder<'_> {
    fn module_factory(&self, group: &FactoryGroup<'_, '_>) -> ModuleFactory {
        let factory_span = group.factory.span();
        let (scope, has_directive) = match group.factory.without_parentheses() {
            Expression::FunctionExpression(func) => (
                func.scope_id.get(),
                func.body.as_ref().is_some_and(|body| has_use_strict(body)),
            ),
            Expression::ArrowFunctionExpression(arrow) => (arrow.scope_id.get(), arrow.has_use_strict_directive()),
            _ => (None, false),
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
        };
        if scan.context.is_some() {
            scan.visit_expression(group.factory);
        }
        ModuleFactory {
            ids: group.ids.clone(),
            start: factory_span.start as usize,
            end: factory_span.end as usize,
            strict,
            exports: scan.exports,
            hoisted: scan.hoisted,
            dependencies: scan.dependencies,
        }
    }
}

fn string_list(expr: &Expression<'_>) -> Vec<String> {
    let Expression::ArrayExpression(list) = expr.without_parentheses() else {
        return Vec::new();
    };
    list.elements
        .iter()
        .filter_map(ArrayExpressionElement::as_expression)
        .filter_map(module_id)
        .collect()
}

impl<'a> Visit<'a> for RegistrationFinder<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let Some(found) = registration(call) else {
            walk::walk_call_expression(self, call);
            return;
        };
        let mut record = ChunkRegistration {
            callee: text(self.source, call.callee.span()),
            script: text(self.source, found.script.span()),
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
}

/// Reads the `__turbopack_context__` calls of one factory.
struct FactoryScan<'s> {
    scoping: &'s Scoping,
    context: Option<SymbolId>,
    exports: Vec<String>,
    hoisted: Vec<HoistedModule>,
    dependencies: Vec<String>,
}

impl FactoryScan<'_> {
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
            Some("i" | "r" | "A") => {
                if let Some(id) = first_module_id(&call.arguments, 0)
                    && !self.dependencies.contains(&id)
                {
                    self.dependencies.push(id);
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
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
    /// stub per scope-hoisted module under `hoisted/`. Each module file is a
    /// chunk registering just that factory, under all of its ids, exactly as
    /// the original chunk did.
    #[must_use]
    pub fn module_files(&self, source: &str) -> Vec<(String, String)> {
        let mut names = BTreeMap::new();
        let mut out = Vec::new();
        let mut stubs = Vec::new();
        for registration in &self.registrations {
            for module in &registration.modules {
                let file = unique_name(&mut names, format!("module_{}.js", safe_component(module.id())));
                out.push((file.clone(), module_file(source, registration, module)));
                for hoisted in &module.hoisted {
                    let stub = unique_name(&mut names, format!("hoisted/module_{}.js", safe_component(&hoisted.id)));
                    stubs.push((stub, hoisted_stub(hoisted, module, &file)));
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
        for module in self.modules() {
            for dep in &module.dependencies {
                let _ = writeln!(graph, "  \"{}\" -> \"{}\";", dot_escape(module.id()), dot_escape(dep));
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
    if module.strict {
        out.push_str("\"use strict\";\n");
    }
    let ids: Vec<String> = module.ids.iter().map(|id| id_literal(id)).collect();
    let _ = writeln!(
        out,
        "{}([{}, {}, {}]);",
        registration.callee,
        registration.script,
        ids.join(", "),
        &source[module.range()]
    );
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

fn safe_component(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim_matches('_');
    if trimmed.is_empty() {
        "anonymous".to_string()
    } else {
        trimmed.to_string()
    }
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
    fn ignores_other_bundles() {
        assert!(detect_turbopack_chunk("(self.webpackChunk=self.webpackChunk||[]).push([[1],{1(e){}}]);").is_none());
        assert!(detect_turbopack_chunk("var a = [1, function(){}]; a.push([2, e=>{}]);").is_none());
    }
}
