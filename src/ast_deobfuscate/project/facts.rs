//! The cross-file facts of one file, read from its original source.
//!
//! They are its ES module links, runtime helpers, the names the single-file
//! pipeline would give its exported locals, and Turbopack / webpack module ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::BuildHasher;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, ExportDefaultDeclarationKind, Expression, ImportDeclarationSpecifier, ImportExpression,
    ModuleExportName, Program, Statement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use super::{TurbopackFacts, import_bindings, specifier_exports};
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::{
    BunHelper, BunRuntimeCollector, ClassNameCollector, DisplayNameCollector, ExportNameCollector, rename_guards,
};

/// What other files of the project need to know about one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    /// `module`, `script` or `commonjs`.
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<ExportFact>,
    /// The runtime helper or name of exported locals, by local name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_names: BTreeMap<String, LocalName>,
    /// Runtime helpers the file defines, by canonical name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub helpers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbopack: Option<TurbopackFacts>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webpack_modules: Vec<String>,
    /// Free (global) names the file reads, which its bindings cannot take.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub globals: Vec<String>,
    /// With `--keep-fn-names`: the exported locals the file's own pipeline
    /// keeps, chiefly those a function or class takes its `.name` from (see
    /// [`crate::ast_deobfuscate::rename_guards`]).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub kept_locals: BTreeSet<String>,
}

/// One module request of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFact {
    pub source: String,
    #[serde(flatten)]
    pub kind: ImportKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportKind {
    /// `import { imported as local }`, `import local from` (`default`).
    Named { imported: String, local: String },
    /// `import * as local from`.
    Namespace { local: String },
    /// `import "source"`.
    SideEffect,
    /// `import("source")`.
    Dynamic,
    /// `export { imported as exported } from`.
    ReExport { imported: String, exported: String },
    /// `export * from` (`exported` is `None`) and `export * as exported from`.
    ReExportAll { exported: Option<String> },
}

/// `export { local as exported }`, `export function local`, `export default local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFact {
    pub exported: String,
    pub local: String,
}

/// What the single-file pipeline knows about an exported local.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalName {
    /// [`BunHelper::canonical`] name of the runtime helper it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// Its `__export` name, class name (`this.name`, `displayName`) or
    /// default-export name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Collect the facts of `code`, parsed as `source_type`.
///
/// `import_helpers` tells which imports, by `(source, imported name)`, are
/// runtime helpers, so that calls to an imported `__export` are read too.
/// `keep_fn_names` is the option the file will be deobfuscated with.
#[must_use]
pub fn collect_facts<S: BuildHasher + Sync>(
    code: &str,
    source_type: SourceType,
    import_helpers: &HashMap<(String, String), BunHelper, S>,
    keep_fn_names: bool,
) -> FileFacts {
    crate::stack::run_on_sized_stack(code.len(), || {
        collect_on_current_stack(code, source_type, import_helpers, keep_fn_names)
    })
}

/// Only the module links of `code` (imports, re-exports, local exports).
#[must_use]
pub fn collect_links(code: &str, source_type: SourceType) -> FileFacts {
    crate::stack::run_on_sized_stack(code.len(), || {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, source_type).parse().program;
        let mut facts = FileFacts {
            source_type: source_type_name(program.source_type).to_string(),
            ..FileFacts::default()
        };
        module_links(&program, &mut facts);
        facts
    })
}

fn source_type_name(source_type: SourceType) -> &'static str {
    if source_type.is_module() {
        "module"
    } else if source_type.is_commonjs() {
        "commonjs"
    } else {
        "script"
    }
}

fn collect_on_current_stack<S: BuildHasher>(
    code: &str,
    source_type: SourceType,
    import_helpers: &HashMap<(String, String), BunHelper, S>,
    keep_fn_names: bool,
) -> FileFacts {
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, source_type).parse().program;
    let mut facts = FileFacts {
        source_type: source_type_name(program.source_type).to_string(),
        ..FileFacts::default()
    };
    module_links(&program, &mut facts);

    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let exported = exported_locals(&program, &scoping);
    facts.globals = scoping
        .root_unresolved_references()
        .keys()
        .map(ToString::to_string)
        .collect();
    facts.globals.sort_unstable();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    let mut runtime = BunRuntimeCollector::new();
    traverse_mut_with_ctx(&mut runtime, &mut program, &mut ctx);
    let helpers = runtime.helper_symbols();
    let mut defined: Vec<String> = helpers.values().map(|h| h.canonical().to_string()).collect();
    defined.sort_unstable();
    defined.dedup();
    facts.helpers = defined;

    if !exported.is_empty() {
        let export_helpers: FxHashSet<SymbolId> = helpers
            .iter()
            .filter(|(_, helper)| **helper == BunHelper::Export)
            .map(|(symbol, _)| *symbol)
            .chain(
                import_bindings(&program)
                    .filter(|(source, imported, _)| {
                        import_helpers.get(&((*source).to_string(), (*imported).to_string()))
                            == Some(&BunHelper::Export)
                    })
                    .map(|(_, _, symbol)| symbol),
            )
            .collect();
        let mut export_names = ExportNameCollector::new(export_helpers);
        traverse_mut_with_ctx(&mut export_names, &mut program, &mut ctx);
        let mut names = export_names.into_map();
        let mut classes = ClassNameCollector::new();
        traverse_mut_with_ctx(&mut classes, &mut program, &mut ctx);
        let mut display_names = DisplayNameCollector::new();
        traverse_mut_with_ctx(&mut display_names, &mut program, &mut ctx);
        for (symbol, name) in classes.into_map().into_iter().chain(display_names.into_map()) {
            names.entry(symbol).or_insert(name);
        }
        for (local, symbol) in &exported {
            let entry = LocalName {
                helper: helpers.get(symbol).map(|helper| helper.canonical().to_string()),
                name: names.get(symbol).cloned(),
            };
            if entry != LocalName::default() {
                facts.local_names.insert(local.clone(), entry);
            }
        }
    }
    drop(ctx);
    if keep_fn_names && !exported.is_empty() {
        // What Phase 0.84 keeps: the guarded symbols but the specifier
        // exports it may rename.
        let guards = rename_guards::guards(&allocator, &mut program, true);
        facts.kept_locals = exported
            .into_iter()
            .filter(|(_, symbol)| guards.symbols.contains(symbol) && !guards.specifier_exports.contains(symbol))
            .map(|(local, _)| local)
            .collect();
    }

    facts.turbopack = crate::ast_deobfuscate::turbopack::project_facts(&allocator, &mut program);
    facts.webpack_modules = crate::ast_deobfuscate::webpack::project_module_ids(&allocator, &mut program);
    facts
}

/// The locals the file exports by name, with their symbols.
fn exported_locals(program: &Program<'_>, scoping: &Scoping) -> Vec<(String, SymbolId)> {
    let mut locals: Vec<(String, SymbolId)> = specifier_exports(program, scoping)
        .map(|(_, symbol)| (scoping.symbol_name(symbol).to_string(), symbol))
        .collect();
    for statement in &program.body {
        if let Statement::ExportDefaultDeclaration(export) = statement {
            let symbol = match &export.declaration {
                ExportDefaultDeclarationKind::Identifier(id) => scoping.get_reference(id.reference_id()).symbol_id(),
                ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                    func.id.as_ref().and_then(|id| id.symbol_id.get())
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    class.id.as_ref().and_then(|id| id.symbol_id.get())
                }
                _ => None,
            };
            locals.extend(symbol.map(|symbol| (scoping.symbol_name(symbol).to_string(), symbol)));
        }
    }
    locals.sort_unstable();
    locals.dedup();
    locals
}

/// Static imports, re-exports, local exports and literal dynamic imports.
fn module_links(program: &Program<'_>, facts: &mut FileFacts) {
    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(import) => {
                let source = import.source.value.to_string();
                let Some(specifiers) = &import.specifiers else {
                    facts.imports.push(ImportFact {
                        source,
                        kind: ImportKind::SideEffect,
                    });
                    continue;
                };
                if specifiers.is_empty() {
                    facts.imports.push(ImportFact {
                        source: source.clone(),
                        kind: ImportKind::SideEffect,
                    });
                }
                for specifier in specifiers {
                    let kind = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => ImportKind::Named {
                            imported: s.imported.name().to_string(),
                            local: s.local.name.to_string(),
                        },
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => ImportKind::Named {
                            imported: "default".to_string(),
                            local: s.local.name.to_string(),
                        },
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => ImportKind::Namespace {
                            local: s.local.name.to_string(),
                        },
                    };
                    facts.imports.push(ImportFact {
                        source: source.clone(),
                        kind,
                    });
                }
            }
            Statement::ExportFromDeclaration(export) => {
                for spec in &export.specifiers {
                    facts.imports.push(ImportFact {
                        source: export.source.value.to_string(),
                        kind: ImportKind::ReExport {
                            imported: spec.local.name().to_string(),
                            exported: spec.exported.name().to_string(),
                        },
                    });
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    if let ModuleExportName::IdentifierReference(local) = &spec.local {
                        facts.exports.push(ExportFact {
                            exported: spec.exported.name().to_string(),
                            local: local.name.to_string(),
                        });
                    }
                }
            }
            Statement::ExportDeclaration(export) => {
                for name in declared_names(&export.declaration) {
                    facts.exports.push(ExportFact {
                        exported: name.clone(),
                        local: name,
                    });
                }
            }
            Statement::ExportAllDeclaration(export) => facts.imports.push(ImportFact {
                source: export.source.value.to_string(),
                kind: ImportKind::ReExportAll {
                    exported: export.exported.as_ref().map(|name| name.name().to_string()),
                },
            }),
            Statement::ExportDefaultDeclaration(export) => {
                let local = match &export.declaration {
                    ExportDefaultDeclarationKind::Identifier(id) => Some(id.name.to_string()),
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        func.id.as_ref().map(|id| id.name.to_string())
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        class.id.as_ref().map(|id| id.name.to_string())
                    }
                    _ => None,
                };
                if let Some(local) = local {
                    facts.exports.push(ExportFact {
                        exported: "default".to_string(),
                        local,
                    });
                }
            }
            _ => {}
        }
    }
    let mut dynamic = DynamicImports::default();
    dynamic.visit_program(program);
    facts
        .imports
        .extend(dynamic.sources.into_iter().map(|source| ImportFact {
            source,
            kind: ImportKind::Dynamic,
        }));
}

fn declared_names(declaration: &Declaration<'_>) -> Vec<String> {
    match declaration {
        Declaration::VariableDeclaration(var) => var
            .declarations
            .iter()
            .flat_map(|declarator| declarator.id.get_binding_identifiers())
            .map(|id| id.name.to_string())
            .collect(),
        _ => declaration.id().map(|id| id.name.to_string()).into_iter().collect(),
    }
}

/// `import("literal")` sources.
#[derive(Default)]
struct DynamicImports {
    sources: Vec<String>,
}

impl<'a> Visit<'a> for DynamicImports {
    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        match &it.source {
            Expression::StringLiteral(source) => self.sources.push(source.value.to_string()),
            Expression::TemplateLiteral(source) if source.expressions.is_empty() => {
                if let Some(cooked) = source.quasis.first().and_then(|q| q.value.cooked.as_ref()) {
                    self.sources.push(cooked.to_string());
                }
            }
            _ => {}
        }
        walk::walk_import_expression(self, it);
    }
}
