use super::{
    Allocator, AstDeobfuscator, BunHelperMap, BunModuleHandleCollector, BunModuleHandleRenamer, BunRuntimeCollector,
    BunRuntimeRenamer, ClassNameCollector, ClassNameRenamer, DeobfuscateState, DisplayNameCollector,
    ExportNameCollector, ExportNameRenamer, ParamHeuristicRenamer, Program, ReusableTraverseCtx, SemanticBuilder, bun,
    flatten_top_level_cjs_wrapper, mem_ck, rename_guards, traverse_mut_with_ctx,
};

impl AstDeobfuscator {
    pub(super) fn run_bun_naming_phases<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
    ) -> BunHelperMap {
        mem_ck("enter Phase 0.9 (helpers)");
        eprintln!("[DEOBFUSCATE] Phase 0.9: Bun runtime helper detection & rename");
        let bun_helper_map = {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut collector = BunRuntimeCollector::new();
            traverse_mut_with_ctx(&mut collector, program, &mut ctx);
            let map = collector.into_map();
            if map.is_empty() {
                eprintln!("[DEOBFUSCATE] Phase 0.9: No Bun helpers detected (not a Bun bundle)");
            } else {
                eprintln!("[DEOBFUSCATE] Phase 0.9: Detected {} Bun runtime helpers", map.len());
                let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
                let mut renamer = BunRuntimeRenamer::new(map.clone());
                traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.9: Renamed {} declarations, {} references",
                    renamer.renamed_decls(),
                    renamer.renamed_refs()
                );
            }
            map
        };

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

        eprintln!("[DEOBFUSCATE] Phase 0.95: Bun __export() name extraction & rename");
        {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut collector = ExportNameCollector::new();
            traverse_mut_with_ctx(&mut collector, program, &mut ctx);
            drop(ctx);
            let mut export_map = collector.into_map();

            // Soundness guard: never rename a binding observable to a direct
            // `eval` or exposed as a native ES export (see `rename_guards`).
            if !export_map.is_empty() {
                let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
                let mut guard = rename_guards::RenameGuardCollector::new();
                traverse_mut_with_ctx(&mut guard, program, &mut ctx);
                drop(ctx);
                let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
                let guarded = guard.into_guarded(&scoping);
                export_map.retain(|sym, _| !guarded.contains(sym));
            }

            if export_map.is_empty() {
                eprintln!("[DEOBFUSCATE] Phase 0.95: No __export() calls found");
            } else {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.95: Found {} export name mappings",
                    export_map.len()
                );
                let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
                // Make assigned names legal + unique-per-scope to avoid invalid JS
                // (reserved words like `instanceof`, duplicate `ZodType`/`Client`).
                let export_map = bun::export_renamer::make_names_unique(export_map, &scoping);
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

        mem_ck("enter Phase 0.99 (module handles)");
        eprintln!("[DEOBFUSCATE] Phase 0.99: Bun module-handle renaming");
        {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut collector = BunModuleHandleCollector::new();
            traverse_mut_with_ctx(&mut collector, program, &mut ctx);
            let module_handles = collector.into_map();
            if module_handles.is_empty() {
                eprintln!("[DEOBFUSCATE] Phase 0.99: No Bun module wrapper handles found");
            } else {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.99: Found {} Bun module wrapper handles",
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

        bun_helper_map
    }
}
