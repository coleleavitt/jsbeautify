//! Project mode (`jsbeautify -d --project <dir> -o <outdir>`): deobfuscate the
//! files of one code-split build together.
//!
//! 1. [`index_project`] reads every JavaScript file in parallel and collects
//!    its [`FileFacts`]: ES module links between chunks, the runtime helpers
//!    each chunk defines, the names the single-file pipeline would give its
//!    exported locals, and Turbopack / webpack module ids.
//! 2. [`plan_project`] resolves every `import { a as e } from "./chunk.js"`
//!    (relative to the importing file, through re-exports) to the binding it
//!    links and gives that binding one name in every file: the runtime helper
//!    it is, the name its own file gives it, its real export name, or else
//!    `<chunk>_<export>`. Exports nobody in the project imports are the public
//!    surface: their short names are names, while the one- to three-character
//!    names chunks use only to reach each other are not. Mangled Turbopack
//!    keys are named by the `esmReexport` lists of every chunk.
//! 3. Each file is deobfuscated by the normal pipeline with its
//!    [`ProjectSeeds`] (see [`crate::ast_deobfuscate::project`]). Import and
//!    export names never change, so links between chunks stay valid.
//! 4. [`check_links`] re-reads the outputs: every export name is kept, every
//!    import resolves to an export of its target, and linked bindings carry
//!    the same name on both ends.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::ParseGoal;
use crate::ast_deobfuscate::BunHelper;
use crate::ast_deobfuscate::bun::export_renamer::names::{is_reserved_word, is_valid_identifier};
use crate::ast_deobfuscate::default_export::is_minified;
pub use crate::ast_deobfuscate::project::facts::{ExportFact, FileFacts, ImportFact, ImportKind, LocalName};
use crate::ast_deobfuscate::project::facts::{collect_facts, collect_links};
pub use crate::ast_deobfuscate::project::{ExportSeed, ImportSeed, ProjectSeeds, TurbopackReexport};

/// Whether `path` is a JavaScript file project mode deobfuscates.
#[must_use]
pub fn is_javascript(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("js" | "mjs" | "cjs"))
}

/// Every file under `root`, relative to it, sorted.
///
/// # Errors
/// Returns an error if a directory cannot be read.
pub fn list_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&dir))? {
            let entry = entry?;
            let relative = dir.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                pending.push(relative);
            } else {
                files.push(relative);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Run `work` on every item on all cores, keeping the order.
///
/// # Panics
/// Panics if `work` panics.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        done.push((index, work(item)));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("project worker panicked"))
            .collect()
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// One JavaScript file of the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFile {
    /// Path relative to the project root, with `/` separators.
    pub path: String,
    pub facts: FileFacts,
}

/// Read the facts of `files` (relative to `root`) in parallel. `goal` gives
/// each file's parse goal.
///
/// # Errors
/// Returns an error if a file cannot be read as UTF-8.
pub fn index_project(
    root: &Path,
    files: &[PathBuf],
    goal: impl Fn(&Path) -> ParseGoal + Sync,
    isolated: bool,
) -> std::io::Result<Vec<IndexedFile>> {
    let sources: Vec<(PathBuf, String)> = files
        .iter()
        .map(|file| Ok((file.clone(), std::fs::read_to_string(root.join(file))?)))
        .collect::<std::io::Result<_>>()?;
    let source_types = parallel_map(&sources, |(path, code)| goal(path).resolve(code, isolated));
    let no_helpers = FxHashMap::default();
    let mut indexed: Vec<IndexedFile> = parallel_map(
        &sources.iter().zip(&source_types).collect::<Vec<_>>(),
        |&((path, code), &source_type)| IndexedFile {
            path: slash_path(path),
            facts: collect_facts(code, source_type, &no_helpers),
        },
    );
    // A chunk that calls an imported `__export` names its exports only once
    // the helper is known: read those chunks again with their imported helpers.
    let graph = Graph::new(&indexed);
    let import_helpers: Vec<FxHashMap<(String, String), BunHelper>> =
        (0..indexed.len()).map(|file| graph.imported_helpers(file)).collect();
    let again: Vec<usize> = (0..indexed.len())
        .filter(|&file| !import_helpers[file].is_empty())
        .collect();
    let reread = parallel_map(&again, |&file| {
        collect_facts(&sources[file].1, source_types[file], &import_helpers[file])
    });
    for (file, facts) in again.into_iter().zip(reread) {
        indexed[file].facts = facts;
    }
    Ok(indexed)
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Where a name for a linked binding came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameSource {
    /// The runtime helper the binding is.
    Helper,
    /// Its `__export` name, class name or default-export name.
    Collector,
    /// Its own unminified name.
    Local,
    /// Its export name.
    ExportName,
    /// `<chunk>_<export>`.
    Synthetic,
}

/// The binding an import or export resolves to: a local of some file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Origin {
    file: usize,
    local: String,
}

/// Resolved module links between the indexed files.
struct Graph<'f> {
    files: &'f [IndexedFile],
    by_path: FxHashMap<&'f str, usize>,
    /// Files whose exports anything may read: namespace or dynamic imports,
    /// `export *`.
    opaque: FxHashSet<usize>,
    /// `(file, export name)` imported by name somewhere.
    imported: FxHashSet<(usize, String)>,
    idents: Vec<String>,
    /// Informative names other files re-export a binding under.
    aliases: FxHashMap<Origin, BTreeSet<String>>,
}

impl<'f> Graph<'f> {
    fn new(files: &'f [IndexedFile]) -> Self {
        let by_path = files.iter().enumerate().map(|(i, f)| (f.path.as_str(), i)).collect();
        let mut graph = Self {
            files,
            by_path,
            opaque: FxHashSet::default(),
            imported: FxHashSet::default(),
            idents: file_idents(files),
            aliases: FxHashMap::default(),
        };
        for (file, indexed) in files.iter().enumerate() {
            for import in &indexed.facts.imports {
                let Some(target) = graph.resolve(file, &import.source) else {
                    continue;
                };
                match &import.kind {
                    ImportKind::Named { imported, .. } | ImportKind::ReExport { imported, .. } => {
                        graph.imported.insert((target, imported.clone()));
                    }
                    ImportKind::Namespace { .. } | ImportKind::Dynamic | ImportKind::ReExportAll { .. } => {
                        graph.opaque.insert(target);
                    }
                    ImportKind::SideEffect => {}
                }
            }
        }
        graph.aliases = graph.collect_aliases();
        graph
    }

    /// `import { a as e } from "./x.js"; export { e as Button }` and
    /// `export { a as Button } from "./x.js"`: `Button` names the binding
    /// behind `./x.js`'s `a`.
    fn collect_aliases(&self) -> FxHashMap<Origin, BTreeSet<String>> {
        let mut aliases: FxHashMap<Origin, BTreeSet<String>> = FxHashMap::default();
        for (file, indexed) in self.files.iter().enumerate() {
            let reexports = indexed
                .facts
                .exports
                .iter()
                .filter_map(|e| Some((self.named_import(file, &e.local)?, &e.exported)));
            let forwarded = indexed.facts.imports.iter().filter_map(|import| match &import.kind {
                ImportKind::ReExport { imported, exported } => {
                    Some(((import.source.as_str(), imported.as_str()), exported))
                }
                _ => None,
            });
            for ((source, imported), exported) in reexports.chain(forwarded) {
                if !self.is_informative_export(file, exported) {
                    continue;
                }
                if let Some(origin) = self
                    .resolve(file, source)
                    .and_then(|target| self.resolve_export(target, imported, 0))
                {
                    aliases.entry(origin).or_default().insert(exported.clone());
                }
            }
        }
        aliases
    }

    /// The file a relative module specifier of `from` names.
    fn resolve(&self, from: usize, specifier: &str) -> Option<usize> {
        let specifier = specifier.split(['?', '#']).next()?;
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return None;
        }
        let mut parts: Vec<&str> = self.files[from].path.split('/').collect();
        parts.pop();
        for component in Path::new(specifier).components() {
            match component {
                Component::ParentDir => {
                    parts.pop()?;
                }
                Component::Normal(part) => parts.push(part.to_str()?),
                _ => {}
            }
        }
        let path = parts.join("/");
        [
            path.clone(),
            format!("{path}.js"),
            format!("{path}.mjs"),
            format!("{path}/index.js"),
        ]
        .iter()
        .find_map(|candidate| self.by_path.get(candidate.as_str()).copied())
    }

    fn named_import<'s>(&'s self, file: usize, local: &str) -> Option<(&'s str, &'s str)> {
        self.files[file]
            .facts
            .imports
            .iter()
            .find_map(|import| match &import.kind {
                ImportKind::Named { imported, local: l } if l == local => {
                    Some((import.source.as_str(), imported.as_str()))
                }
                _ => None,
            })
    }

    fn is_namespace_import(&self, file: usize, local: &str) -> bool {
        self.files[file]
            .facts
            .imports
            .iter()
            .any(|import| matches!(&import.kind, ImportKind::Namespace { local: l } if l == local))
    }

    /// The local binding export `name` of `file` resolves to.
    fn resolve_export(&self, file: usize, name: &str, depth: usize) -> Option<Origin> {
        if depth > 32 {
            return None;
        }
        let facts = &self.files[file].facts;
        if let Some(export) = facts.exports.iter().find(|export| export.exported == name) {
            if let Some((source, imported)) = self.named_import(file, &export.local) {
                return self.resolve_export(self.resolve(file, source)?, imported, depth + 1);
            }
            if self.is_namespace_import(file, &export.local) {
                return None;
            }
            return Some(Origin {
                file,
                local: export.local.clone(),
            });
        }
        for import in &facts.imports {
            if let ImportKind::ReExport { imported, exported } = &import.kind
                && exported == name
            {
                return self.resolve_export(self.resolve(file, &import.source)?, imported, depth + 1);
            }
        }
        if name == "default" {
            return None;
        }
        facts.imports.iter().find_map(|import| match &import.kind {
            ImportKind::ReExportAll { exported: None } => {
                self.resolve_export(self.resolve(file, &import.source)?, name, depth + 1)
            }
            _ => None,
        })
    }

    /// Whether `file` exports `name` (directly, re-exported, or via `export *`).
    fn has_export(&self, file: usize, name: &str, depth: usize) -> bool {
        let facts = &self.files[file].facts;
        depth <= 32
            && (facts.exports.iter().any(|e| e.exported == name)
                || facts.imports.iter().any(|import| match &import.kind {
                    ImportKind::ReExport { exported, .. }
                    | ImportKind::ReExportAll {
                        exported: Some(exported),
                    } => exported == name,
                    ImportKind::ReExportAll { exported: None } => {
                        name != "default"
                            && self
                                .resolve(file, &import.source)
                                .is_some_and(|target| self.has_export(target, name, depth + 1))
                    }
                    _ => false,
                }))
    }

    /// `import { imported as local } from source` of `file` that are runtime
    /// helpers, by `(source, imported)`.
    fn imported_helpers(&self, file: usize) -> FxHashMap<(String, String), BunHelper> {
        self.files[file]
            .facts
            .imports
            .iter()
            .filter_map(|import| {
                let ImportKind::Named { imported, .. } = &import.kind else {
                    return None;
                };
                let origin = self.resolve_export(self.resolve(file, &import.source)?, imported, 0)?;
                let helper = self.local_name(&origin)?.helper.as_deref()?;
                Some((
                    (import.source.clone(), imported.clone()),
                    BunHelper::from_canonical(helper)?,
                ))
            })
            .collect()
    }

    fn local_name(&self, origin: &Origin) -> Option<&LocalName> {
        self.files[origin.file].facts.local_names.get(&origin.local)
    }

    /// Nothing in the project can read `name` of `file`: its short name is
    /// public, not a link between chunks.
    fn is_public(&self, file: usize, name: &str) -> bool {
        !self.opaque.contains(&file) && !self.imported.contains(&(file, name.to_string()))
    }

    /// Whether export `name` of `file` can name its local.
    fn is_informative_export(&self, file: usize, name: &str) -> bool {
        if !is_valid_identifier(name) || is_reserved_word(name) || name == "default" {
            return false;
        }
        // One to three characters is what bundlers give the exports chunks
        // use to reach each other.
        name.chars().count() > 3 || self.is_public(file, name)
    }

    /// The name of `origin` in every file, without a synthetic fallback.
    fn origin_name(&self, origin: &Origin) -> Option<(String, NameSource)> {
        if let Some(info) = self.local_name(origin) {
            if let Some(helper) = &info.helper {
                return Some((helper.clone(), NameSource::Helper));
            }
            if let Some(name) = info.name.as_ref().filter(|name| !is_generic_default_name(name)) {
                return Some((name.clone(), NameSource::Collector));
            }
        }
        if !is_minified(&origin.local) && !origin.local.starts_with("_0x") {
            return Some((origin.local.clone(), NameSource::Local));
        }
        let mut names: Vec<&str> = self.export_names(origin).collect();
        names.sort_unstable();
        names
            .into_iter()
            .find(|name| self.is_informative_export(origin.file, name))
            .or_else(|| self.aliases.get(origin)?.first().map(String::as_str))
            .map(|name| (name.to_string(), NameSource::ExportName))
    }

    fn export_names<'s>(&'s self, origin: &'s Origin) -> impl Iterator<Item = &'s str> + 's {
        self.files[origin.file]
            .facts
            .exports
            .iter()
            .filter(move |export| export.local == origin.local)
            .map(|export| export.exported.as_str())
    }

    /// [`Self::origin_name`], or `<chunk>_<export>` for a minified binding.
    fn linked_name(&self, origin: &Origin) -> Option<(String, NameSource)> {
        self.origin_name(origin).or_else(|| {
            let export = self.export_names(origin).min()?;
            let name = format!("{}_{export}", self.idents[origin.file]);
            (is_minified(&origin.local) && is_valid_identifier(&name)).then_some((name, NameSource::Synthetic))
        })
    }

    /// The names of the minified locals `file` exports, by local: their
    /// [`Self::linked_name`] when another file can read them, else their
    /// [`Self::origin_name`]. Two locals that want one name get `$N`, as the
    /// exporting file's own rename would give them, so importers agree.
    fn export_local_names(&self, file: usize) -> FxHashMap<String, (String, NameSource)> {
        let facts = &self.files[file].facts;
        let mut locals: Vec<&str> = facts
            .exports
            .iter()
            .map(|export| export.local.as_str())
            .filter(|local| is_minified(local))
            .filter(|local| self.named_import(file, local).is_none() && !self.is_namespace_import(file, local))
            .collect();
        locals.sort_unstable();
        locals.dedup();
        let mut taken: FxHashSet<String> = facts
            .exports
            .iter()
            .map(|e| e.local.clone())
            .chain(facts.globals.iter().cloned())
            .collect();
        let mut names = FxHashMap::default();
        for local in locals {
            let origin = Origin {
                file,
                local: local.to_string(),
            };
            let linked = self.export_names(&origin).any(|name| !self.is_public(file, name));
            let name = if linked {
                self.linked_name(&origin)
            } else {
                self.origin_name(&origin)
            };
            let Some((mut name, source)) = name else { continue };
            let base = name.clone();
            let mut n = 0;
            while taken.contains(&name) {
                n += 1;
                name = format!("{base}${n}");
            }
            taken.insert(name.clone());
            names.insert(local.to_string(), (name, source));
        }
        names
    }
}

/// `default_export` / `default_3`: the fallback names of a default export,
/// which say nothing across files (`<chunk>_default` does).
fn is_generic_default_name(name: &str) -> bool {
    name == "default_export"
        || name
            .strip_prefix("default_")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// A short identifier per file: the file name without its content hash or a
/// `chunk-` prefix, camel-cased; the full name where two files would share one.
fn file_idents(files: &[IndexedFile]) -> Vec<String> {
    let short: Vec<String> = files.iter().map(|f| file_ident(&f.path, true)).collect();
    let mut counts: FxHashMap<&str, usize> = FxHashMap::default();
    for ident in &short {
        *counts.entry(ident.as_str()).or_default() += 1;
    }
    files
        .iter()
        .zip(&short)
        .map(|(f, ident)| {
            if counts[ident.as_str()] == 1 {
                ident.clone()
            } else {
                file_ident(&f.path, false)
            }
        })
        .collect()
}

fn file_ident(path: &str, strip: bool) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if strip {
        // `[name]-[hash]` with an 8-character hash (vite, rollup, esbuild).
        let cut = stem.len().wrapping_sub(9);
        if stem.len() > 9 && stem.is_char_boundary(cut) && stem.as_bytes()[cut] == b'-' {
            stem = &stem[..cut];
        }
        if let Some(rest) = stem.strip_prefix("chunk-").or_else(|| stem.strip_prefix("chunk.")) {
            stem = rest;
        }
    }
    let mut ident = String::new();
    let mut upper = false;
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '$' {
            if upper && !ident.is_empty() {
                ident.extend(c.to_uppercase());
            } else {
                ident.push(c);
            }
            upper = false;
        } else {
            upper = true;
        }
    }
    if ident.is_empty() || ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident.insert_str(0, "chunk");
    }
    ident
}

/// Counts for the project index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanStats {
    pub files: usize,
    pub modules: usize,
    pub scripts: usize,
    pub commonjs: usize,
    /// `import { … }` bindings and `export { … } from` re-exports.
    pub named_imports: usize,
    /// Of those, the ones whose target file and binding were found.
    pub resolved_imports: usize,
    /// Relative module specifiers no project file matches.
    pub unresolved_specifiers: BTreeSet<String>,
    /// Files that define runtime helpers.
    pub helper_files: usize,
    /// Import bindings identified as runtime helpers of another chunk.
    pub imported_helpers: usize,
    /// Import bindings named by the plan, by where the name came from.
    pub imports_named: BTreeMap<NameSource, usize>,
    /// Minified exported locals named by the plan, by where the name came from.
    pub exports_named: BTreeMap<NameSource, usize>,
    /// Exports no project file can read (the public surface).
    pub public_exports: usize,
    /// Exports other chunks read.
    pub internal_exports: usize,
    pub turbopack_files: usize,
    pub turbopack_mangled_exports: usize,
    /// Mangled keys an `esmReexport` in the same chunk names.
    pub turbopack_named_in_file: usize,
    /// Mangled keys only another chunk's `esmReexport` names.
    pub turbopack_named_cross_file: usize,
    /// `(module, key)` read under two names, left unnamed.
    pub turbopack_conflicting_keys: usize,
    /// Turbopack module ids registered by more than one chunk.
    pub turbopack_shared_modules: usize,
    pub webpack_files: usize,
    pub webpack_modules: usize,
}

/// The naming plan: one [`ProjectSeeds`] per indexed file, in order.
pub struct ProjectPlan {
    pub seeds: Vec<ProjectSeeds>,
    pub stats: PlanStats,
}

/// Build the project naming plan of `files`.
#[must_use]
pub fn plan_project(files: &[IndexedFile]) -> ProjectPlan {
    let graph = Graph::new(files);
    let names: Vec<FxHashMap<String, (String, NameSource)>> =
        (0..files.len()).map(|file| graph.export_local_names(file)).collect();
    let mut stats = PlanStats {
        files: files.len(),
        ..PlanStats::default()
    };
    let mut seeds = vec![ProjectSeeds::default(); files.len()];
    for (file, indexed) in files.iter().enumerate() {
        let facts = &indexed.facts;
        match facts.source_type.as_str() {
            "module" => stats.modules += 1,
            "commonjs" => stats.commonjs += 1,
            _ => stats.scripts += 1,
        }
        stats.helper_files += usize::from(!facts.helpers.is_empty());
        for import in &facts.imports {
            let (imported, local) = match &import.kind {
                ImportKind::Named { imported, local } => (imported, Some(local)),
                ImportKind::ReExport { imported, .. } => (imported, None),
                ImportKind::Namespace { .. }
                | ImportKind::SideEffect
                | ImportKind::Dynamic
                | ImportKind::ReExportAll { .. } => {
                    if import.source.starts_with('.') && graph.resolve(file, &import.source).is_none() {
                        stats
                            .unresolved_specifiers
                            .insert(format!("{}: {}", indexed.path, import.source));
                    }
                    continue;
                }
            };
            stats.named_imports += 1;
            let Some(target) = graph.resolve(file, &import.source) else {
                if import.source.starts_with('.') {
                    stats
                        .unresolved_specifiers
                        .insert(format!("{}: {}", indexed.path, import.source));
                }
                continue;
            };
            let Some(origin) = graph.resolve_export(target, imported, 0) else {
                continue;
            };
            stats.resolved_imports += 1;
            let Some(local) = local else { continue };
            let helper = graph.local_name(&origin).and_then(|info| info.helper.clone());
            let unminified = (!is_minified(&origin.local)).then(|| (origin.local.clone(), NameSource::Local));
            let name = names[origin.file]
                .get(&origin.local)
                .or(unminified.as_ref())
                .filter(|(name, _)| is_minified(local) && name != local);
            stats.imported_helpers += usize::from(helper.is_some());
            if let Some((_, source)) = name {
                *stats.imports_named.entry(*source).or_default() += 1;
            }
            if name.is_some() || helper.is_some() {
                seeds[file].imports.push(ImportSeed {
                    source: import.source.clone(),
                    imported: imported.clone(),
                    name: name.map(|(name, _)| name.clone()),
                    helper,
                });
            }
        }
        for export in &facts.exports {
            if graph.is_public(file, &export.exported) {
                stats.public_exports += 1;
            } else {
                stats.internal_exports += 1;
            }
            if let Some((name, source)) = names[file].get(&export.local)
                && *name != export.local
            {
                *stats.exports_named.entry(*source).or_default() += 1;
                seeds[file].exports.push(ExportSeed {
                    exported: export.exported.clone(),
                    name: name.clone(),
                });
            }
        }
    }
    plan_turbopack(files, &mut seeds, &mut stats);
    for indexed in files {
        stats.webpack_files += usize::from(!indexed.facts.webpack_modules.is_empty());
        stats.webpack_modules += indexed.facts.webpack_modules.len();
    }
    ProjectPlan { seeds, stats }
}

/// Name mangled Turbopack export keys through every chunk's `esmReexport`
/// lists: one name per `(module id, key)` in the whole project.
fn plan_turbopack(files: &[IndexedFile], seeds: &mut [ProjectSeeds], stats: &mut PlanStats) {
    let mut names: BTreeMap<(&str, &str), Option<&str>> = BTreeMap::new();
    let mut registered: FxHashMap<&str, usize> = FxHashMap::default();
    for facts in files.iter().filter_map(|f| f.facts.turbopack.as_ref()) {
        stats.turbopack_files += 1;
        for id in &facts.module_ids {
            *registered.entry(id.as_str()).or_default() += 1;
        }
        for reexport in &facts.reexports {
            let name = reexport.name.as_deref();
            names
                .entry((reexport.module.as_str(), reexport.key.as_str()))
                .and_modify(|known| {
                    if *known != name {
                        *known = None;
                    }
                })
                .or_insert(name);
        }
    }
    stats.turbopack_conflicting_keys = names.values().filter(|name| name.is_none()).count();
    stats.turbopack_shared_modules = registered.values().filter(|&&count| count > 1).count();
    for (file, indexed) in files.iter().enumerate() {
        let Some(facts) = &indexed.facts.turbopack else {
            continue;
        };
        let own: FxHashSet<(&str, &str)> = facts
            .reexports
            .iter()
            .filter(|r| r.name.is_some())
            .map(|r| (r.module.as_str(), r.key.as_str()))
            .collect();
        let mut seeded = FxHashSet::default();
        for mangled in &facts.mangled_exports {
            stats.turbopack_mangled_exports += 1;
            if mangled
                .modules
                .iter()
                .any(|m| own.contains(&(m.as_str(), mangled.key.as_str())))
            {
                stats.turbopack_named_in_file += 1;
                continue;
            }
            let Some((module, name)) = mangled
                .modules
                .iter()
                .find_map(|module| Some((module, (*names.get(&(module.as_str(), mangled.key.as_str()))?)?)))
            else {
                continue;
            };
            stats.turbopack_named_cross_file += 1;
            if seeded.insert((module.as_str(), mangled.key.as_str())) {
                seeds[file].turbopack_reexports.push(TurbopackReexport {
                    module: module.clone(),
                    key: mangled.key.clone(),
                    name: name.to_string(),
                });
            }
        }
    }
}

/// What [`check_links`] found in the deobfuscated files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkReport {
    /// Files whose export names differ from the input's.
    pub changed_exports: Vec<String>,
    /// Named imports checked against their target's exports.
    pub imports_checked: usize,
    /// `file: import name from source` whose target lacks the export.
    pub broken_imports: Vec<String>,
    /// Imports whose local name equals the name of the binding it links.
    pub consistent_names: usize,
    /// Imports whose local name differs from the linked binding's name
    /// (a `$N` collision suffix, or a binding no file could name).
    pub inconsistent_names: usize,
    /// The first of them: `file: local = source#export (local there)`.
    pub inconsistent_samples: Vec<String>,
}

/// Check the deobfuscated `outputs` (same order as `inputs`): export names
/// unchanged, every named import resolves to an export of its target, and
/// linked bindings carry one name.
#[must_use]
pub fn check_links(
    inputs: &[IndexedFile],
    outputs: &[(String, String)],
    goal: impl Fn(&Path) -> ParseGoal + Sync,
) -> LinkReport {
    let after: Vec<IndexedFile> = parallel_map(outputs, |(path, code)| IndexedFile {
        path: path.clone(),
        facts: collect_links(code, goal(Path::new(path)).resolve(code, false)),
    });
    let graph = Graph::new(&after);
    let mut report = LinkReport::default();
    for (file, (before, now)) in inputs.iter().zip(&after).enumerate() {
        let names = |facts: &FileFacts| -> BTreeSet<String> {
            facts
                .exports
                .iter()
                .map(|e| e.exported.clone())
                .chain(facts.imports.iter().filter_map(|i| match &i.kind {
                    ImportKind::ReExport { exported, .. } => Some(exported.clone()),
                    ImportKind::ReExportAll { exported } => {
                        Some(exported.clone().unwrap_or_else(|| format!("* from {}", i.source)))
                    }
                    _ => None,
                }))
                .collect()
        };
        if names(&before.facts) != names(&now.facts) {
            report.changed_exports.push(now.path.clone());
        }
        for import in &now.facts.imports {
            let ImportKind::Named { imported, local } = &import.kind else {
                continue;
            };
            let Some(target) = graph.resolve(file, &import.source) else {
                continue;
            };
            report.imports_checked += 1;
            if !graph.has_export(target, imported, 0) {
                report
                    .broken_imports
                    .push(format!("{}: {imported} from {}", now.path, import.source));
                continue;
            }
            match graph.resolve_export(target, imported, 0) {
                Some(origin) if origin.local == *local => report.consistent_names += 1,
                Some(origin) => {
                    report.inconsistent_names += 1;
                    if report.inconsistent_samples.len() < 50 {
                        report.inconsistent_samples.push(format!(
                            "{}: {local} = {}#{imported} ({})",
                            now.path, import.source, origin.local
                        ));
                    }
                }
                None => {}
            }
        }
    }
    report
}

#[cfg(test)]
mod tests;
