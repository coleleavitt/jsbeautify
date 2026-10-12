use super::{
    Allocator, AstDeobfuscator, DeobfuscateState, DispatchInlinerCollector, DispatchInlinerRewriter,
    DoWhileSwitchCleaner, DoWhileSwitchDetector, FunctionCollector, FunctionInliner, FxHashMap, Program,
    ReusableTraverseCtx, TraceSpan, collect_constants, eliminate_dead_vars, mem_ck, rename_guards,
    resolve_compound_transitions, traverse_mut_with_ctx,
};
use crate::ast_deobfuscate::dead_var_elimination::DeadVarSweepResult;
use crate::ast_deobfuscate::semantic;
use oxc_semantic::Scoping;

impl AstDeobfuscator {
    /// Phases 8.7–12.5. `scoping` is a current analysis of `program`, if the
    /// previous phases left one.
    pub(super) fn run_structural_late<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
        scoping: Option<Scoping>,
    ) {
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
        let scoping = semantic::reuse_or_build(program, scoping);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut func_collector = FunctionCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 9: Running function_collector");
        traverse_mut_with_ctx(&mut func_collector, program, &mut ctx);
        let single_use = func_collector.get_single_use_functions();
        let found = single_use.len();
        let mut scoping = ctx.into_scoping();
        let rewrites = if single_use.is_empty() {
            0
        } else {
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = FunctionInliner::new(single_use);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            scoping = semantic::refresh(program, ctx.into_scoping(), true);
            inliner.rewrites()
        };
        eprintln!("[DEOBFUSCATE] Phase 9: Found {found} single-use functions, {rewrites} inline rewrites");

        eprintln!("[DEOBFUSCATE] Phase 9.5: dispatch inliner (string-array-factory resolution)");
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut di_collector = DispatchInlinerCollector::new();
        traverse_mut_with_ctx(&mut di_collector, program, &mut ctx);
        let mut scoping = ctx.into_scoping();
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
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut di_rewriter = DispatchInlinerRewriter::new(di_factories, di_constants);
            traverse_mut_with_ctx(&mut di_rewriter, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 9.5: Inlined {} dispatch call sites",
                di_rewriter.inlined()
            );
            scoping = semantic::refresh(program, ctx.into_scoping(), true);
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
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            traverse_mut_with_ctx(&mut cleaner, program, &mut ctx);
            eprintln!("[DEOBFUSCATE] Phase 9.7: Pruned {} dead cases", cleaner.pruned_cases());
            scoping = semantic::refresh(program, ctx.into_scoping(), true);
        }

        let scoping = self.run_sweeps_and_simplifiers(allocator, program, scoping);

        {
            let _trace = TraceSpan::new("Phase 12: variable_renamer");
            eprintln!("[DEOBFUSCATE] Phase 12: SemanticBuilder for variable_renamer");
            let mut scoping = semantic::reuse_or_build(program, scoping);
            let mut kept = if self.keep_fn_names {
                let guards;
                (guards, scoping) = rename_guards::guards_with(allocator, program, scoping, true);
                guards.symbols
            } else {
                rustc_hash::FxHashSet::default()
            };
            kept.extend(crate::ast_deobfuscate::project::seeded_imports(program, &self.project));
            self.variable_renamer.keep(kept);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            eprintln!("[DEOBFUSCATE] Phase 12: Running variable_renamer");
            traverse_mut_with_ctx(&mut self.variable_renamer, program, &mut ctx);
            eprintln!("[DEOBFUSCATE] Phase 12: DONE");
        }

        self.run_local_slot_rename(allocator, program);
    }

    /// Phases 10.5–11.5: the dead-var sweeps around the Phase 10 and 11
    /// simplifiers. Returns the scoping of the program they leave when the last
    /// sweep left it unchanged.
    fn run_sweeps_and_simplifiers<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
        scoping: Scoping,
    ) -> Option<Scoping> {
        mem_ck("enter Phase 10.5 (dead-var sweep)");
        let (sweep, scoping) = Self::run_dead_var_sweep(
            allocator,
            program,
            "10.5",
            "Post-CFF dead-var sweep",
            Some(scoping),
            None,
        );

        eprintln!("[DEOBFUSCATE] Phase 10: SemanticBuilder for array/dynamic/ternary/try_catch");
        let scoping = semantic::reuse_or_build(program, scoping);
        let rewrites_before = self.phase_10_rewrites();
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
        let unchanged = self.phase_10_rewrites() == rewrites_before;
        let scoping = unchanged.then(|| ctx.into_scoping());

        let (sweep, _) = Self::run_dead_var_sweep(
            allocator,
            program,
            "10.7",
            "Post-Phase-10 dead-var sweep",
            scoping,
            unchanged.then_some(sweep),
        );

        // Rebuild scoping after try_catch_remover and dead_code_eliminator
        // which create new BlockStatement nodes during traversal.
        mem_ck("enter Phase 11 (bool/void/sparsing)");
        eprintln!("[DEOBFUSCATE] Phase 11: SemanticBuilder for boolean/void/object_sparsing");
        let semantic_result = semantic::build(program);
        eprintln!(
            "[DEOBFUSCATE] Phase 11: SemanticBuilder completed, errors: {}",
            semantic_result.diagnostics.len()
        );
        for (i, err) in semantic_result.diagnostics.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] Phase 11: Semantic error {i}: {err:?}");
        }
        let scoping = semantic_result.semantic.into_scoping();
        let rewrites_before = self.phase_11_rewrites();
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
        let unchanged = self.phase_11_rewrites() == rewrites_before;
        let scoping = unchanged.then(|| ctx.into_scoping());

        Self::run_dead_var_sweep(
            allocator,
            program,
            "11.5",
            "Post-Phase-11 dead-var sweep",
            scoping,
            unchanged.then_some(sweep),
        )
        .1
    }

    const fn phase_10_rewrites(&self) -> usize {
        self.array_unpacker.rewrites()
            + self.dynamic_property_converter.rewrites()
            + self.ternary_simplifier.rewrites()
            + self.try_catch_remover.rewrites()
    }

    const fn phase_11_rewrites(&self) -> usize {
        self.boolean_literal_converter.rewrites()
            + self.void_replacer.rewrites()
            + self.object_sparsing_consolidator.rewrites()
    }

    /// One dead-var sweep. `scoping` is a current analysis of `program`, if
    /// one is at hand; `previous` is the result of the last sweep when the
    /// program has not changed since, and a sweep that removed nothing then is
    /// not run again. Returns the result and, when the sweep left the program
    /// unchanged, its scoping.
    pub(super) fn run_dead_var_sweep<'a>(
        allocator: &'a Allocator,
        program: &mut Program<'a>,
        phase: &str,
        label: &str,
        scoping: Option<Scoping>,
        previous: Option<DeadVarSweepResult>,
    ) -> (DeadVarSweepResult, Option<Scoping>) {
        eprintln!("[DEOBFUSCATE] Phase {phase}: {label}");
        let (result, scoping) = match previous.filter(|result| result.removed_declarators() == 0) {
            Some(result) => (result, scoping),
            None => eliminate_dead_vars(allocator, program, semantic::reuse_or_build(program, scoping)),
        };
        eprintln!(
            "[DEOBFUSCATE] Phase {phase}: Removed {} dead declarators ({} candidate symbols, rejected={})",
            result.removed_declarators(),
            result.candidate_symbols(),
            result.was_rejected()
        );
        (result, scoping)
    }
}
