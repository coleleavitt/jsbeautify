use super::{
    Allocator, AstDeobfuscator, BunHelper, BunModuleHandleCollector, BunModuleHandleRenamer, BunRuntimeCollector,
    BunRuntimeRenamer, BundleRuntime, Bundler, ClassNameCollector, ClassNameRenamer, DeobfuscateState,
    DisplayNameCollector, ExportNameCollector, ExportNameRenamer, ParamHeuristicRenamer, Program, ReusableTraverseCtx,
    SemanticBuilder, bun, flatten_top_level_cjs_wrapper, mem_ck, rename_guards, traverse_mut_with_ctx,
};

use oxc_ast::ast::Ident;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

impl AstDeobfuscator {
    pub(super) fn run_bun_naming_phases<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
    ) -> BundleRuntime {
        let runtime = rename_runtime_helpers(allocator, program);

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
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut collector = BunRuntimeCollector::new();
            traverse_mut_with_ctx(&mut collector, program, &mut ctx);
            collector.helper_symbols().clone()
        };

        rename_exports(allocator, program, &helper_symbols);

        eprintln!("[DEOBFUSCATE] Phase 0.97: Class name detection (this.name / displayName)");
        {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut class_collector = ClassNameCollector::new();
            traverse_mut_with_ctx(&mut class_collector, program, &mut ctx);
            let mut class_map = class_collector.into_map();

            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut display_collector = DisplayNameCollector::new();
            traverse_mut_with_ctx(&mut display_collector, program, &mut ctx);
            for (k, v) in display_collector.into_map() {
                class_map.entry(k).or_insert(v);
            }

            if !class_map.is_empty() {
                let guarded = rename_guards::guarded_symbols(allocator, program);
                class_map.retain(|sym, _| !guarded.contains(sym));
            }
            if !class_map.is_empty() {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.97: Found {} class/component names",
                    class_map.len()
                );
                let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
                let mut renamer = ClassNameRenamer::new(class_map);
                traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
                eprintln!("[DEOBFUSCATE] Phase 0.97: Renamed {} identifiers", renamer.renamed());
            }
        }

        eprintln!("[DEOBFUSCATE] Phase 0.98: Heuristic parameter renaming");
        {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut param_renamer = ParamHeuristicRenamer::new();
            traverse_mut_with_ctx(&mut param_renamer, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.98: Renamed {} parameters",
                param_renamer.renamed()
            );
        }

        rename_module_handles(allocator, program, helper_symbols);

        runtime
    }
}

/// Phase 0.9: detects the esbuild / Bun runtime helpers and renames them to their canonical names.
fn rename_runtime_helpers<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> BundleRuntime {
    mem_ck("enter Phase 0.9 (helpers)");
    eprintln!("[DEOBFUSCATE] Phase 0.9: bundler runtime helper detection & rename");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = BunRuntimeCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    drop(ctx);
    let mut helper_symbols = collector.helper_symbols().clone();
    let runtime = collector.into_runtime();
    if runtime.helpers.is_empty() {
        eprintln!("[DEOBFUSCATE] Phase 0.9: No bundler runtime helpers detected");
    } else {
        let mut helpers: Vec<_> = runtime.helpers.iter().collect();
        helpers.sort_unstable_by_key(|(name, _)| *name);
        for (name, helper) in helpers {
            eprintln!("[DEOBFUSCATE] Phase 0.9: {name} = {}", helper.canonical());
        }
        eprintln!(
            "[DEOBFUSCATE] Phase 0.9: Detected {} runtime helpers, bundler: {}",
            runtime.helpers.len(),
            runtime.bundler.map_or("unknown", Bundler::name)
        );
        let guards = rename_guards::guards(allocator, program);
        helper_symbols.retain(|symbol, _| !guards.symbols.contains(symbol));
        let desired = helper_symbols
            .into_iter()
            .map(|(symbol, helper)| (symbol, helper.canonical().to_string()))
            .collect();
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let globals: FxHashSet<&str> = scoping.root_unresolved_references().keys().map(Ident::as_str).collect();
        let reserved = guards.eval_names.iter().map(String::as_str).collect();
        let mut plan = bun::export_renamer::make_names_unique(desired, &scoping, &reserved);
        plan.retain(|_, name| !globals.contains(name.as_str()));
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut renamer = BunRuntimeRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.9: Renamed {} declarations, {} references",
            renamer.renamed_decls(),
            renamer.renamed_refs()
        );
    }

    runtime
}

/// Phase 0.95: renames the locals passed to the `__export` helper to their export names.
fn rename_exports<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    helper_symbols: &FxHashMap<SymbolId, BunHelper>,
) {
    eprintln!("[DEOBFUSCATE] Phase 0.95: __export() name extraction & rename");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let export_helpers = helper_symbols
        .iter()
        .filter(|(_, helper)| **helper == BunHelper::Export)
        .map(|(symbol, _)| *symbol)
        .collect();
    let mut collector = ExportNameCollector::new(export_helpers);
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    drop(ctx);
    let mut export_map = collector.into_map();

    // Soundness guard: never rename a binding observable to a direct
    // `eval`, exposed as a native ES export, or a classic-script global
    // (see `rename_guards`).
    let eval_names = if export_map.is_empty() {
        FxHashSet::default()
    } else {
        let guards = rename_guards::guards(allocator, program);
        export_map.retain(|sym, _| !guards.symbols.contains(sym));
        guards.eval_names
    };

    if export_map.is_empty() {
        eprintln!("[DEOBFUSCATE] Phase 0.95: No __export() calls found");
    } else {
        eprintln!(
            "[DEOBFUSCATE] Phase 0.95: Found {} export name mappings",
            export_map.len()
        );
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        // Legal, unique-per-scope names (reserved words like `instanceof`,
        // duplicate `ZodType`/`Client`) that never capture a global or a
        // name read by `eval("name")`.
        let reserved = scoping
            .root_unresolved_references()
            .keys()
            .map(Ident::as_str)
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
) {
    mem_ck("enter Phase 0.99 (module handles)");
    eprintln!("[DEOBFUSCATE] Phase 0.99: module-handle renaming");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
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
        let guards = rename_guards::guards(allocator, program);
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
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
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
