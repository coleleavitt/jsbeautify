//! Cross-file naming for the files of one code-split build (`jsbeautify -d
//! --project`).
//!
//! A file alone cannot tell what its imports are: `import { a as e } from
//! "./chunk.js"` may be esbuild's `__export` helper, and a Turbopack chunk's
//! mangled export key may be named only by another chunk's `esmReexport`
//! list. The project driver ([`crate::project`]) reads every file once
//! ([`facts::collect_facts`]), builds one naming plan for the whole build, and
//! hands each file its part as [`ProjectSeeds`]. The per-file pipeline then:
//!
//! - Phase 0.84 ([`apply_project_seeds`]) renames, by resolved `SymbolId`, the
//!   import bindings and the locals behind `export { local as name }` to the
//!   names the plan gives the bindings they link, so one binding has one name
//!   in every file. Import and export names never change, so every link
//!   between files stays as it was. The shared rename guards apply, except
//!   that a local exported by a specifier may be renamed (the specifier keeps
//!   the public name); collisions get `$N`.
//! - Phases 0.9–0.99 treat imported runtime helpers ([`seeded_helpers`]) like
//!   local ones, so `__export(…)` calls and `__commonJS` / `__esm` wrappers
//!   that use an imported helper are named.
//! - Phase 0.86 resolves mangled Turbopack keys through the `esmReexport`
//!   names of every chunk ([`ProjectSeeds::turbopack_reexports`]).

pub mod facts;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ImportDeclarationSpecifier, ModuleExportName, Program, Statement};
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::ast_deobfuscate::rename_planner::rename_unguarded;
use crate::ast_deobfuscate::{BunHelper, rename_guards};

/// One file's part of the project naming plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSeeds {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportSeed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<ExportSeed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turbopack_reexports: Vec<TurbopackReexport>,
}

impl ProjectSeeds {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.imports.is_empty() && self.exports.is_empty() && self.turbopack_reexports.is_empty()
    }
}

/// `import { <imported> as local } from "<source>"`: the name for `local`,
/// and the runtime helper the import is, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSeed {
    pub source: String,
    pub imported: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// [`BunHelper::canonical`] name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
}

/// `export { local as <exported> }`: the name for `local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportSeed {
    pub exported: String,
    pub name: String,
}

/// The export name behind Turbopack module `module`'s key `key`, from an
/// `esmReexport` list in some chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurbopackReexport {
    pub module: String,
    pub key: String,
    pub name: String,
}

/// What a Turbopack chunk tells other chunks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurbopackFacts {
    /// Ids of the module factories the chunk registers.
    pub module_ids: Vec<String>,
    /// `esmReexport` names: `name` is `None` when the chunk reads the key
    /// under two names.
    pub reexports: Vec<TurbopackKeyName>,
    /// Exports under mangled keys, which only an `esmReexport` can name.
    pub mangled_exports: Vec<TurbopackMangledExport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurbopackKeyName {
    pub module: String,
    pub key: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurbopackMangledExport {
    pub modules: Vec<String>,
    pub key: String,
}

/// The import bindings of `program`: `(source, imported name, local symbol)`,
/// with `default` for `import x from`. Namespace imports have no imported name.
pub(crate) fn import_bindings<'p>(program: &'p Program<'_>) -> impl Iterator<Item = (&'p str, &'p str, SymbolId)> {
    program.body.iter().flat_map(|statement| {
        let Statement::ImportDeclaration(import) = statement else {
            return Vec::new();
        };
        let source = import.source.value.as_str();
        import
            .specifiers
            .iter()
            .flatten()
            .filter_map(|specifier| match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    Some((source, s.imported.name().as_str(), s.local.symbol_id.get()?))
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    Some((source, "default", s.local.symbol_id.get()?))
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => None,
            })
            .collect()
    })
}

/// The locals of `export { local as name }`: `(name, local symbol)`.
pub(crate) fn specifier_exports<'p>(
    program: &'p Program<'_>,
    scoping: &'p Scoping,
) -> impl Iterator<Item = (&'p str, SymbolId)> {
    program.body.iter().flat_map(move |statement| {
        let Statement::ExportNamedDeclaration(export) = statement else {
            return Vec::new();
        };
        export
            .specifiers
            .iter()
            .filter_map(|spec| {
                let ModuleExportName::IdentifierReference(local) = &spec.local else {
                    return None;
                };
                let symbol = scoping.get_reference(local.reference_id()).symbol_id()?;
                Some((spec.exported.name().as_str(), symbol))
            })
            .collect()
    })
}

/// The import bindings `seeds` identify as runtime helpers. `program`'s
/// binding symbols must be those of the latest semantic build.
pub(crate) fn seeded_helpers(program: &Program<'_>, seeds: &ProjectSeeds) -> FxHashMap<SymbolId, BunHelper> {
    let helpers: FxHashMap<(&str, &str), BunHelper> = seeds
        .imports
        .iter()
        .filter_map(|seed| {
            let helper = BunHelper::from_canonical(seed.helper.as_deref()?)?;
            Some(((seed.source.as_str(), seed.imported.as_str()), helper))
        })
        .collect();
    if helpers.is_empty() {
        return FxHashMap::default();
    }
    import_bindings(program)
        .filter_map(|(source, imported, symbol)| Some((symbol, *helpers.get(&(source, imported))?)))
        .collect()
}

/// Phase 0.84: rename the import bindings and specifier-exported locals
/// `seeds` name. Returns the number of renamed bindings.
pub(crate) fn apply_project_seeds<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    seeds: &ProjectSeeds,
) -> usize {
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let imports: FxHashMap<(&str, &str), &str> = seeds
        .imports
        .iter()
        .filter_map(|seed| Some(((seed.source.as_str(), seed.imported.as_str()), seed.name.as_deref()?)))
        .collect();
    let exports: FxHashMap<&str, &str> = seeds
        .exports
        .iter()
        .map(|seed| (seed.exported.as_str(), seed.name.as_str()))
        .collect();
    let mut planned = FxHashSet::default();
    let mut plan: Vec<(SymbolId, String)> = import_bindings(program)
        .filter_map(|(source, imported, symbol)| Some((symbol, (*imports.get(&(source, imported))?).to_string())))
        .chain(
            specifier_exports(program, &scoping)
                .filter_map(|(exported, symbol)| Some((symbol, (*exports.get(exported)?).to_string()))),
        )
        .filter(|(symbol, _)| planned.insert(*symbol))
        .collect();
    drop(scoping);
    if plan.is_empty() {
        return 0;
    }

    let guards = rename_guards::guards(allocator, program);
    plan.retain(|(_, name)| !guards.eval_names.contains(name));
    let guarded: FxHashSet<SymbolId> = guards.symbols.difference(&guards.specifier_exports).copied().collect();
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let counts = rename_unguarded(allocator, program, scoping, plan, &guarded);
    eprintln!(
        "[DEOBFUSCATE] Phase 0.84: Renamed {} bindings from the project plan ({} declarations, {} references)",
        counts.planned, counts.declarations, counts.references
    );
    counts.planned
}

#[cfg(test)]
mod tests;
