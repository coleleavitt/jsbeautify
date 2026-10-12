use super::{
    Allocator, AstDeobfuscator, DeobfuscateState, Program, ReusableTraverseCtx, TraceSpan, bun, deob_trace_enabled,
    heuristic_rename, mem_ck, rename_guards, traverse_mut_with_ctx,
};
use crate::ast_deobfuscate::semantic;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

impl AstDeobfuscator {
    pub(super) fn run_local_slot_rename<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        mem_ck("enter Phase 12.5 (local slot rename)");
        eprintln!("[DEOBFUSCATE] Phase 12.5: Bun local slot renaming (single-char locals -> v0,v1,...)");
        let _trace = TraceSpan::new("Phase 12.5: Bun local slot renaming");
        let guard = Self::collect_rename_guards(allocator, program, self.keep_fn_names);
        let guarded = {
            let _trace = TraceSpan::new("Phase 12.5: resolve rename guards");
            let scoping = semantic::scoping(program);
            let mut guarded = guard.into_guarded(&scoping);
            guarded.extend(crate::ast_deobfuscate::project::seeded_imports(program, &self.project));
            guarded
        };
        let (desired, free_names) = if self.readable_names {
            Self::collect_readable_name_inputs(allocator, program)
        } else {
            Default::default()
        };
        let mode = if self.readable_names {
            bun::local_renamer::RenameMode::Readable
        } else {
            bun::local_renamer::RenameMode::Slot
        };
        let (plan, plan_stats) = {
            let _trace = TraceSpan::new("Phase 12.5: plan local renames");
            let scoping = semantic::scoping(program);
            bun::local_renamer::plan_local_renames_with_stats(&scoping, &guarded, mode, &desired, &free_names)
        };
        if deob_trace_enabled() {
            eprintln!("[DEOBFUSCATE] Phase 12.5: plan stats {plan_stats:?}");
        }
        if plan.is_empty() {
            eprintln!("[DEOBFUSCATE] Phase 12.5: No single-char locals to rename");
        } else {
            self.apply_local_rename_plan(allocator, program, plan, guarded.len());
        }
    }

    fn collect_rename_guards<'a>(
        allocator: &'a Allocator,
        program: &mut Program<'a>,
        keep_fn_names: bool,
    ) -> rename_guards::RenameGuardCollector {
        let _trace = TraceSpan::new("Phase 12.5: collect rename guards");
        let scoping = semantic::scoping(program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut guard = rename_guards::RenameGuardCollector::new().keep_fn_names(keep_fn_names);
        traverse_mut_with_ctx(&mut guard, program, &mut ctx);
        guard
    }

    fn collect_readable_name_inputs<'a>(
        allocator: &'a Allocator,
        program: &mut Program<'a>,
    ) -> (FxHashMap<SymbolId, String>, FxHashMap<ScopeId, FxHashSet<String>>) {
        let _trace = TraceSpan::new("Phase 12.5: collect readable-name heuristics");
        let scoping = semantic::scoping(program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut collector = heuristic_rename::HeuristicNameCollector::new();
        traverse_mut_with_ctx(&mut collector, program, &mut ctx);
        collector.into_parts()
    }

    fn apply_local_rename_plan<'a>(
        &mut self,
        allocator: &'a Allocator,
        program: &mut Program<'a>,
        plan: bun::local_renamer::LocalRenameMap,
        guarded_len: usize,
    ) {
        let _trace = TraceSpan::new("Phase 12.5: apply local renames");
        let plan_len = plan.len();
        let scoping = semantic::scoping(program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut renamer = bun::local_renamer::LocalRenamer::new(plan);
        traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 12.5: Renamed {} single-char local bindings to slot names ({} refs, {} function ids, {} planned, {} guarded)",
            renamer.renamed(),
            renamer.renamed_refs(),
            renamer.renamed_functions(),
            plan_len,
            guarded_len
        );
    }
}
