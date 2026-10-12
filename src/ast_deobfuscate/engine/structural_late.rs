use super::{
    Allocator, AstDeobfuscator, DeobfuscateState, DispatchInlinerCollector, DispatchInlinerRewriter,
    DoWhileSwitchCleaner, DoWhileSwitchDetector, FunctionCollector, FunctionInliner, FxHashMap, Program,
    ReusableTraverseCtx, SemanticBuilder, TraceSpan, collect_constants, eliminate_dead_vars, mem_ck, rename_guards,
    resolve_compound_transitions, traverse_mut_with_ctx,
};

impl AstDeobfuscator {
    pub(super) fn run_structural_late<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 8.7: do-while-switch detector");
        let dowhile_detector = DoWhileSwitchDetector::new();
        let mut dowhile_dispatchers = dowhile_detector.detect(program);
        eprintln!(
            "[DEOBFUSCATE] Phase 8.7: Found {} do-while-switch dispatchers",
            dowhile_dispatchers.len()
        );
        for (name, info) in &dowhile_dispatchers {
            deob_trace!(
                "[DOWHILE] found {}({}, {}) with {} cases, exit_sentinel={}",
                name,
                info.state_param,
                info.args_param,
                info.cases.len(),
                info.exit_sentinel
            );
        }

        mem_ck("enter Phase 9 (function inline)");
        eprintln!("[DEOBFUSCATE] Phase 9: SemanticBuilder for function_inline");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut func_collector = FunctionCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 9: Running function_collector");
        traverse_mut_with_ctx(&mut func_collector, program, &mut ctx);
        let single_use = func_collector.get_single_use_functions();
        let found = single_use.len();
        let rewrites = if single_use.is_empty() {
            0
        } else {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = FunctionInliner::new(single_use);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            inliner.rewrites()
        };
        eprintln!("[DEOBFUSCATE] Phase 9: Found {found} single-use functions, {rewrites} inline rewrites");

        eprintln!("[DEOBFUSCATE] Phase 9.5: dispatch inliner (string-array-factory resolution)");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut di_collector = DispatchInlinerCollector::new();
        traverse_mut_with_ctx(&mut di_collector, program, &mut ctx);
        let (di_factories, di_constants) = di_collector.into_maps();
        eprintln!(
            "[DEOBFUSCATE] Phase 9.5: Found {} string-array factories, {} index constants",
            di_factories.len(),
            di_constants.len()
        );
        let di_constants_i64: FxHashMap<String, i64> = di_constants
            .iter()
            .map(|(k, &v)| (k.clone(), i64::try_from(v).unwrap_or(i64::MAX)))
            .collect();
        if !di_factories.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut di_rewriter = DispatchInlinerRewriter::new(di_factories, di_constants);
            traverse_mut_with_ctx(&mut di_rewriter, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 9.5: Inlined {} dispatch call sites",
                di_rewriter.inlined()
            );
        }

        if !dowhile_dispatchers.is_empty() {
            let own_constants = collect_constants(program);
            let mut merged = di_constants_i64;
            for (k, v) in own_constants {
                merged.entry(k).or_insert(v);
            }
            let mut total_resolved = 0;
            for info in dowhile_dispatchers.values_mut() {
                total_resolved += resolve_compound_transitions(info, &merged);
            }
            eprintln!("[DEOBFUSCATE] Phase 9.6: Resolved {total_resolved} compound transitions to sequential");

            eprintln!("[DEOBFUSCATE] Phase 9.7: do-while-switch dead-case pruner");
            let mut cleaner = DoWhileSwitchCleaner::new(dowhile_dispatchers, program);
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            traverse_mut_with_ctx(&mut cleaner, program, &mut ctx);
            eprintln!("[DEOBFUSCATE] Phase 9.7: Pruned {} dead cases", cleaner.pruned_cases());
        }

        mem_ck("enter Phase 10.5 (dead-var sweep)");
        Self::run_dead_var_sweep(allocator, program, "10.5", "Post-CFF dead-var sweep");

        eprintln!("[DEOBFUSCATE] Phase 10: SemanticBuilder for array/dynamic/ternary/try_catch");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        eprintln!("[DEOBFUSCATE] Phase 10: Running array_unpacker");
        traverse_mut_with_ctx(&mut self.array_unpacker, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running dynamic_property_converter");
        traverse_mut_with_ctx(&mut self.dynamic_property_converter, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running ternary_simplifier");
        traverse_mut_with_ctx(&mut self.ternary_simplifier, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running try_catch_remover");
        traverse_mut_with_ctx(&mut self.try_catch_remover, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 10: Unpacked {} array accesses, converted {} dynamic properties, simplified {} ternaries, removed {} try/catch wrappers",
            self.array_unpacker.rewrites(),
            self.dynamic_property_converter.rewrites(),
            self.ternary_simplifier.rewrites(),
            self.try_catch_remover.rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 10: DONE");

        Self::run_dead_var_sweep(allocator, program, "10.7", "Post-Phase-10 dead-var sweep");

        // Rebuild scoping after try_catch_remover and dead_code_eliminator
        // which create new BlockStatement nodes during traversal.
        mem_ck("enter Phase 11 (bool/void/sparsing)");
        eprintln!("[DEOBFUSCATE] Phase 11: SemanticBuilder for boolean/void/object_sparsing");
        let semantic_result = SemanticBuilder::new().build(program);
        eprintln!(
            "[DEOBFUSCATE] Phase 11: SemanticBuilder completed, errors: {}",
            semantic_result.diagnostics.len()
        );
        for (i, err) in semantic_result.diagnostics.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] Phase 11: Semantic error {i}: {err:?}");
        }
        let scoping = semantic_result.semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        eprintln!("[DEOBFUSCATE] Phase 11: Running boolean_literal_converter");
        traverse_mut_with_ctx(&mut self.boolean_literal_converter, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: Running void_replacer");
        traverse_mut_with_ctx(&mut self.void_replacer, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: Running object_sparsing_consolidator");
        traverse_mut_with_ctx(&mut self.object_sparsing_consolidator, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 11: Converted {} boolean literals, replaced {} void expressions, consolidated {} objects",
            self.boolean_literal_converter.rewrites(),
            self.void_replacer.rewrites(),
            self.object_sparsing_consolidator.rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 11: DONE");

        Self::run_dead_var_sweep(allocator, program, "11.5", "Post-Phase-11 dead-var sweep");

        {
            let _trace = TraceSpan::new("Phase 12: variable_renamer");
            eprintln!("[DEOBFUSCATE] Phase 12: SemanticBuilder for variable_renamer");
            let mut kept = if self.keep_fn_names {
                rename_guards::guards(allocator, program, true).symbols
            } else {
                rustc_hash::FxHashSet::default()
            };
            kept.extend(crate::ast_deobfuscate::project::seeded_imports(program, &self.project));
            self.variable_renamer.keep(kept);
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            eprintln!("[DEOBFUSCATE] Phase 12: Running variable_renamer");
            traverse_mut_with_ctx(&mut self.variable_renamer, program, &mut ctx);
            eprintln!("[DEOBFUSCATE] Phase 12: DONE");
        }

        self.run_local_slot_rename(allocator, program);
    }

    fn run_dead_var_sweep<'a>(allocator: &'a Allocator, program: &mut Program<'a>, phase: &str, label: &str) {
        eprintln!("[DEOBFUSCATE] Phase {phase}: {label}");
        let result = eliminate_dead_vars(allocator, program);
        eprintln!(
            "[DEOBFUSCATE] Phase {phase}: Removed {} dead declarators ({} candidate symbols, rejected={})",
            result.removed_declarators(),
            result.candidate_symbols(),
            result.was_rejected()
        );
    }
}
