use super::{
    Allocator, AstDeobfuscator, BunHelper, BunModuleHandleCollector, BunModuleHandleRenamer, BunRuntimeCollector,
    BunRuntimeRenamer, BundleRuntime, Bundler, ClassNameCollector, ClassNameRenamer, DeobfuscateState,
    DisplayNameCollector, ExportNameCollector, ExportNameRenamer, ParamHeuristicRenamer, Program, ReusableTraverseCtx,
    bun, flatten_top_level_cjs_wrapper, mem_ck, rename_guards, traverse_mut_with_ctx,
};
use crate::ast_deobfuscate::semantic;

use crate::ast_deobfuscate::project;
use oxc_ast::ast::Ident;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

impl AstDeobfuscator {
    pub(super) fn run_bun_naming_phases<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
    ) -> BundleRuntime {
        let runtime = rename_runtime_helpers(allocator, program, &self.project, self.keep_fn_names);

        eprintln!("[DEOBFUSCATE] Phase 0.91: Bun CJS wrapper flattening");
        let flattened_statements = flatten_top_level_cjs_wrapper(program, allocator);
        if flattened_statements > 0 {
            // The statements now run as the wrapper's function body.
            program.source_type = program.source_type.with_commonjs(true);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.91: Flattened top-level Bun CJS wrapper ({flattened_statements} statements)"
            );
        } else {
            eprintln!("[DEOBFUSCATE] Phase 0.91: No top-level Bun CJS wrapper flattened");
        }

        let helper_symbols = {
            let scoping = semantic::scoping(program);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut collector = BunRuntimeCollector::new();
            traverse_mut_with_ctx(&mut collector, program, &mut ctx);
            let mut helper_symbols = collector.helper_symbols().clone();
            helper_symbols.extend(project::seeded_helpers(program, &self.project));
            helper_symbols
        };

        rename_exports(
            allocator,
            program,
            &helper_symbols,
            &self.project,
            self.keep_fn_names,
            self.short_export_names,
        );

        eprintln!("[DEOBFUSCATE] Phase 0.97: Class name detection (this.name / displayName)");
        {
            let scoping = semantic::scoping(program);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut class_collector = ClassNameCollector::new();
            traverse_mut_with_ctx(&mut class_collector, program, &mut ctx);
            let mut class_map = class_collector.into_map();

            let scoping = semantic::scoping(program);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut display_collector = DisplayNameCollector::new();
            traverse_mut_with_ctx(&mut display_collector, program, &mut ctx);
            for (k, v) in display_collector.into_map() {
                class_map.entry(k).or_insert(v);
            }

            let eval_names = if class_map.is_empty() {
                FxHashSet::default()
            } else {
                let guards = rename_guards::guards(allocator, program, self.keep_fn_names);
                class_map.retain(|sym, _| !guards.symbols.contains(sym));
                guards.eval_names
            };
            if !class_map.is_empty() {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.97: Found {} class/component names",
                    class_map.len()
                );
                let scoping = semantic::scoping(program);
                // Two classes may carry the same `this.name`, and a name may
                // already be bound or read as a global in scope.
                let reserved = rename_guards::free_global_names(program, &scoping)
                    .into_iter()
                    .chain(eval_names.iter().map(String::as_str))
                    .collect();
                let class_map = bun::export_renamer::make_names_unique(class_map, &scoping, &reserved);
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
                let mut renamer = ClassNameRenamer::new(class_map);
                traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
                eprintln!("[DEOBFUSCATE] Phase 0.97: Renamed {} identifiers", renamer.renamed());
            }
        }

        eprintln!("[DEOBFUSCATE] Phase 0.98: Heuristic parameter renaming");
        {
            let scoping = semantic::scoping(program);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut param_renamer = ParamHeuristicRenamer::new().keep_fn_names(self.keep_fn_names);
            traverse_mut_with_ctx(&mut param_renamer, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.98: Renamed {} parameters",
                param_renamer.renamed()
            );
        }

        rename_module_handles(allocator, program, helper_symbols, self.keep_fn_names);

        runtime
    }
}

/// Result of the Phase 0.9 check: which runtime the helpers identify.
fn runtime_check_result(runtime: &BundleRuntime) -> String {
    let helpers = runtime.helpers.len();
    match runtime.bundler {
        _ if helpers == 0 => "not found".to_string(),
        Some(bundler) => format!("detected {} runtime with {helpers} helpers", bundler.name()),
        None => format!("detected {helpers} runtime helpers shared by esbuild and Bun (bundler not identified)"),
    }
}

/// Phase 0.9: detects the esbuild / Bun runtime helpers and renames them to
/// their canonical names. Helpers imported from another chunk of the project
/// (`seeds`) keep the names Phase 0.84 gave them.
fn rename_runtime_helpers<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    seeds: &project::ProjectSeeds,
    keep_fn_names: bool,
) -> BundleRuntime {
    mem_ck("enter Phase 0.9 (helpers)");
    eprintln!("[DEOBFUSCATE] Phase 0.9: esbuild/Bun runtime check");
    let scoping = semantic::scoping(program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = BunRuntimeCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    let scoping = ctx.into_scoping();
    let imported: Vec<(String, BunHelper)> = project::seeded_helpers(program, seeds)
        .into_iter()
        .map(|(symbol, helper)| (scoping.symbol_name(symbol).to_string(), helper))
        .collect();
    drop(scoping);
    let helper_symbols = collector.helper_symbols().clone();
    let mut runtime = collector.into_runtime();
    eprintln!(
        "[DEOBFUSCATE] Phase 0.9: esbuild/Bun runtime check: {}",
        runtime_check_result(&runtime)
    );
    if !runtime.helpers.is_empty() {
        let mut helpers: Vec<_> = runtime.helpers.iter().collect();
        helpers.sort_unstable_by_key(|(name, _)| *name);
        for (name, helper) in helpers {
            eprintln!("[DEOBFUSCATE] Phase 0.9: {name} = {}", helper.canonical());
        }
        let guards = rename_guards::guards(allocator, program, keep_fn_names);
        // A helper a code-split chunk exports (`export { __esm as a }`) keeps its export name.
        let desired = helper_symbols
            .iter()
            .filter(|(symbol, _)| !guards.symbols.contains(symbol) || guards.specifier_exports.contains(symbol))
            .map(|(symbol, helper)| (*symbol, helper.canonical().to_string()))
            .collect();
        let scoping = semantic::scoping(program);
        let globals: FxHashSet<&str> = scoping.root_unresolved_references().keys().map(Ident::as_str).collect();
        let reserved = guards.eval_names.iter().map(String::as_str).collect();
        let mut plan = bun::export_renamer::make_names_unique(desired, &scoping, &reserved);
        plan.retain(|_, name| !globals.contains(name.as_str()));
        // Later phases (Phase 19's module annotation) find the helpers by these names.
        runtime.helpers = helper_symbols
            .iter()
            .map(|(symbol, helper)| {
                let name = plan
                    .get(symbol)
                    .cloned()
                    .unwrap_or_else(|| scoping.symbol_name(*symbol).to_string());
                (name, *helper)
            })
            .collect();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut renamer = BunRuntimeRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.9: Renamed {} declarations, {} references",
            renamer.renamed_decls(),
            renamer.renamed_refs()
        );
    }
    if !imported.is_empty() {
        eprintln!(
            "[DEOBFUSCATE] Phase 0.9: {} runtime helpers imported from other chunks",
            imported.len()
        );
        runtime.helpers.extend(imported);
    }

    runtime
}

/// Phase 0.95: renames the locals passed to the `__export` helper and the
/// minified locals of native `export { local as name }` to their export
/// names, and minified `default` export locals (of `__export` and of native
/// `export default`) per [`crate::ast_deobfuscate::default_export`].
fn rename_exports<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    helper_symbols: &FxHashMap<SymbolId, BunHelper>,
    seeds: &project::ProjectSeeds,
    keep_fn_names: bool,
    short_export_names: bool,
) {
    eprintln!("[DEOBFUSCATE] Phase 0.95: __export() name extraction & rename");
    let scoping = semantic::scoping(program);
    // Import bindings named by the project plan keep its name.
    let seeded = project::seeded_imports(program, seeds);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let export_helpers = helper_symbols
        .iter()
        .filter(|(_, helper)| **helper == BunHelper::Export)
        .map(|(symbol, _)| *symbol)
        .collect();
    let mut collector = ExportNameCollector::new(export_helpers).short_export_names(short_export_names);
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    drop(ctx);
    let (mut export_map, specifier_names) = collector.into_maps();

    // Soundness guard: never rename a binding observable to a direct
    // `eval`, exposed as a native ES export, or a classic-script global
    // (see `rename_guards`). A local exported only by `export { local as name }`
    // may take its export name: the specifier keeps the public name.
    let eval_names = if export_map.is_empty() && specifier_names.is_empty() {
        FxHashSet::default()
    } else {
        let guards = rename_guards::guards(allocator, program, keep_fn_names);
        export_map.retain(|sym, _| !guards.symbols.contains(sym));
        export_map.extend(
            specifier_names
                .into_iter()
                .filter(|(sym, _)| guards.specifier_exports.contains(sym)),
        );
        export_map.retain(|sym, _| !seeded.contains(sym));
        guards.eval_names
    };

    if export_map.is_empty() {
        eprintln!("[DEOBFUSCATE] Phase 0.95: No export names found");
    } else {
        eprintln!(
            "[DEOBFUSCATE] Phase 0.95: Found {} export name mappings",
            export_map.len()
        );
        let scoping = semantic::scoping(program);
        // Legal, unique-per-scope names (reserved words like `instanceof`,
        // duplicate `ZodType`/`Client`) that never capture a global or a
        // name read by `eval("name")`.
        let reserved = rename_guards::free_global_names(program, &scoping)
            .into_iter()
            .chain(eval_names.iter().map(String::as_str))
            .collect();
        let export_map = bun::export_renamer::make_names_unique(export_map, &scoping, &reserved);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut renamer = ExportNameRenamer::new(export_map);
        traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.95: Renamed {} declarations + {} references = {} total",
            renamer.renamed_decls(),
            renamer.renamed_refs(),
            renamer.total_renamed()
        );
    }
}

/// Phase 0.99: names the lazy module wrapper handles `init_N` / `require_N`.
fn rename_module_handles<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    helper_symbols: FxHashMap<SymbolId, BunHelper>,
    keep_fn_names: bool,
) {
    mem_ck("enter Phase 0.99 (module handles)");
    eprintln!("[DEOBFUSCATE] Phase 0.99: module-handle renaming");
    let scoping = semantic::scoping(program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = BunModuleHandleCollector::new(helper_symbols);
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    let mut module_handles = FxHashMap::default();
    if !collector.is_empty() {
        let mut reserved: FxHashSet<String> = ctx
            .into_scoping()
            .root_unresolved_references()
            .keys()
            .map(Ident::to_string)
            .collect();
        let guards = rename_guards::guards(allocator, program, keep_fn_names);
        reserved.extend(guards.eval_names);
        module_handles = collector.into_map(&reserved);
        module_handles.retain(|sym, _| !guards.symbols.contains(sym));
    }
    if module_handles.is_empty() {
        eprintln!("[DEOBFUSCATE] Phase 0.99: No module wrapper handles found");
    } else {
        eprintln!(
            "[DEOBFUSCATE] Phase 0.99: Found {} module wrapper handles",
            module_handles.len()
        );
        let scoping = semantic::scoping(program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut renamer = BunModuleHandleRenamer::new(module_handles);
        traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.99: Renamed {} declarations, {} references",
            renamer.renamed_decls(),
            renamer.renamed_refs()
        );
    }
}
