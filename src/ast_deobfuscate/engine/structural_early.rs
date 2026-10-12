use super::{
    AkamaiDeobfuscator, Allocator, AstDeobfuscator, DeobfuscateState, LookupForwarderCollector, LookupForwarderInliner,
    MethodCallForwarderCollector, MethodCallForwarderInliner, Program, ReusableTraverseCtx, SelfInitAccessorFlattener,
    SemanticBuilder, TrampolineCollector, TrampolineInliner, WindowAliasCollector, WindowAliasPropagator,
    traverse_mut_with_ctx,
};

impl AstDeobfuscator {
    pub(super) fn run_structural_preamble<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        Self::run_window_alias_preamble(allocator, program);

        eprintln!("[DEOBFUSCATE] Phase 0.5: Akamai BMP check");
        let mut akamai = AkamaiDeobfuscator::new();
        if akamai.detect(program) {
            Self::run_akamai_core(allocator, program, &mut akamai);
            Self::run_akamai_forwarders(allocator, program);
            Self::run_akamai_trampolines(allocator, program);
        }
    }

    fn run_window_alias_preamble<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 0.4: Window-alias propagation");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut collector = WindowAliasCollector::new();
        traverse_mut_with_ctx(&mut collector, program, &mut ctx);
        let aliases = collector.aliases();
        if !aliases.is_empty() {
            eprintln!(
                "[DEOBFUSCATE] Phase 0.4: Found {} window alias(es): {:?}",
                aliases.len(),
                aliases
            );
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut propagator = WindowAliasPropagator::new(aliases);
            traverse_mut_with_ctx(&mut propagator, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.4: Propagated {} window-alias accesses",
                propagator.rewrites()
            );
        }
    }

    fn run_akamai_core<'a>(allocator: &'a Allocator, program: &mut Program<'a>, akamai: &mut AkamaiDeobfuscator) {
        eprintln!("[DEOBFUSCATE] Phase 0.5a: Akamai stack-tracker removal");
        akamai.tracker_remover.detect(program);
        if akamai.tracker_remover.tracker_names().is_empty() {
            eprintln!("[DEOBFUSCATE] Phase 0.5a: No pure stack-tracker variable identified");
        } else {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            traverse_mut_with_ctx(&mut akamai.tracker_remover, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5a: Removed {} stack-tracker calls for trackers {:?}",
                akamai.tracker_remover.removed_call_count(),
                akamai.tracker_remover.tracker_names()
            );
        }

        eprintln!("[DEOBFUSCATE] Phase 0.5b: Akamai undefined-pattern + boolean-arithmetic folding");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        traverse_mut_with_ctx(&mut akamai.undef_normalizer, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.5b: Replaced {} [][[]] patterns with undefined",
            akamai.undef_normalizer.replaced_count()
        );
        traverse_mut_with_ctx(&mut akamai.bool_folder, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.5b: Folded {} boolean-arithmetic expressions",
            akamai.bool_folder.folded_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 0.5c: Akamai equality-proxy unwrapping");
        akamai.eq_unwrapper.collect(program);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.5c: Found {} equality-proxy functions",
            akamai.eq_unwrapper.proxies().len()
        );
        if !akamai.eq_unwrapper.proxies().is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            traverse_mut_with_ctx(&mut akamai.eq_unwrapper, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5c: Unwrapped {} equality-proxy call sites",
                akamai.eq_unwrapper.unwrapped_count()
            );
        }
        Self::log_akamai_summary(akamai);

        eprintln!("[DEOBFUSCATE] Phase 0.5d: Akamai self-init-accessor flattener");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut accessor_flattener = SelfInitAccessorFlattener::new();
        traverse_mut_with_ctx(&mut accessor_flattener, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 0.5d: Flattened {} self-init accessors",
            accessor_flattener.rewritten()
        );
    }

    fn log_akamai_summary(akamai: &AkamaiDeobfuscator) {
        eprintln!("[AKAMAI] ─── summary ───");
        eprintln!(
            "[AKAMAI]   trackers detected     : {:?}",
            akamai.tracker_remover.tracker_names()
        );
        eprintln!(
            "[AKAMAI]   tracker calls removed : {}",
            akamai.tracker_remover.removed_call_count()
        );
        eprintln!(
            "[AKAMAI]   [][[]] -> undefined    : {}",
            akamai.undef_normalizer.replaced_count()
        );
        eprintln!(
            "[AKAMAI]   bool-arith folded      : {}",
            akamai.bool_folder.folded_count()
        );
        eprintln!(
            "[AKAMAI]   eq-proxies identified  : {}",
            akamai.eq_unwrapper.proxies().len()
        );
        eprintln!(
            "[AKAMAI]   eq-proxy calls unwrapped: {}",
            akamai.eq_unwrapper.unwrapped_count()
        );
    }

    fn run_akamai_forwarders<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 0.5e: Akamai lookup-forwarder inliner");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut fwd_collector = LookupForwarderCollector::new();
        traverse_mut_with_ctx(&mut fwd_collector, program, &mut ctx);
        let forwarders = fwd_collector.forwarders();
        eprintln!("[DEOBFUSCATE] Phase 0.5e: Found {} lookup forwarders", forwarders.len());
        if !forwarders.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = LookupForwarderInliner::new(forwarders);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5e: Inlined {} lookup-forwarder call sites",
                inliner.inlined()
            );
        }

        eprintln!("[DEOBFUSCATE] Phase 0.5e2: Akamai 2-arg method-call forwarder inliner");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut mc_collector = MethodCallForwarderCollector::new();
        traverse_mut_with_ctx(&mut mc_collector, program, &mut ctx);
        let mc_forwarders = mc_collector.into_forwarders();
        eprintln!(
            "[DEOBFUSCATE] Phase 0.5e2: Found {} method-call forwarders",
            mc_forwarders.len()
        );
        if !mc_forwarders.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = MethodCallForwarderInliner::new(mc_forwarders);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5e2: Inlined {} method-call-forwarder call sites",
                inliner.inlined()
            );
        }
    }

    fn run_akamai_trampolines<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 0.5f: Akamai trampoline inliner");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut tramp_collector = TrampolineCollector::new();
        traverse_mut_with_ctx(&mut tramp_collector, program, &mut ctx);
        let trampolines = tramp_collector.into_trampolines();
        eprintln!("[DEOBFUSCATE] Phase 0.5f: Found {} trampolines", trampolines.len());
        if !trampolines.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = TrampolineInliner::new(trampolines);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5f: Inlined {} trampoline call sites",
                inliner.inlined()
            );
        }
    }
}
