use super::{
    Allocator, ApplyCallSimplifier, AstDeobfuscator, BunSyntaxNormalizer, CallProxyCollector, CallProxyInliner,
    CallThisSimplifier, CffUnflattener, ConcatCanonicaliser, DeobfuscateState, DispatcherDetector, DynConcatResolver,
    FromCharCodeFolder, OperatorProxyCollector, OperatorProxyInliner, Program, ReusableTraverseCtx, SemanticBuilder,
    UnaryProxyCollector, UnaryProxyInliner, collect_case_bodies, eliminate_dead_vars, mem_ck, traverse_mut_with_ctx,
};

impl AstDeobfuscator {
    pub(super) fn run_structural_mid<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        self.run_string_array_and_decoder_phases(allocator, program);
        Self::run_proxy_and_call_simplifiers(allocator, program);
        self.run_expression_cluster(allocator, program);
        self.run_dead_var_and_cff(allocator, program);
    }

    fn run_string_array_and_decoder_phases<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        mem_ck("enter Phase 1 (control-flow unflatten)");
        eprintln!("[DEOBFUSCATE] Phase 1: SemanticBuilder for control_flow_unflattener");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);

        eprintln!("[DEOBFUSCATE] Phase 1: Running control_flow_unflattener");
        traverse_mut_with_ctx(&mut self.control_flow_unflattener, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 1: Unflattened {} control-flow loops",
            self.control_flow_unflattener.rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 1: control_flow_unflattener DONE");

        eprintln!("[DEOBFUSCATE] Phase 2: SemanticBuilder for string_array_rotation");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);

        eprintln!("[DEOBFUSCATE] Phase 2: Running string_array_rotation");
        traverse_mut_with_ctx(&mut self.string_array_rotation, program, &mut ctx);
        let (arrays, rotations) = self.string_array_rotation.detected();
        eprintln!("[DEOBFUSCATE] Phase 2: Found {arrays} string arrays, {rotations} rotation IIFEs");
        eprintln!("[DEOBFUSCATE] Phase 2: string_array_rotation DONE");

        let mut state = ctx.into_state();
        self.string_array_rotation.finalize(&mut state);

        eprintln!("[DEOBFUSCATE] Phase 2.5: Self-defending string array (javascript-obfuscator modern)");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, allocator);
        traverse_mut_with_ctx(&mut self.self_defending_string_array, program, &mut ctx);
        let mut state = ctx.into_state();
        self.self_defending_string_array.finalize(&mut state);
        eprintln!(
            "[DEOBFUSCATE] Phase 2.5: Found {} string arrays, {} decoders",
            state.string_arrays.len(),
            state.decoders.len()
        );

        mem_ck("enter Phase 3 (decoder/string-array)");
        eprintln!("[DEOBFUSCATE] Phase 3: SemanticBuilder for decoder/string_array/dispatcher");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, allocator);

        eprintln!("[DEOBFUSCATE] Phase 3: Running decoder_inliner");
        traverse_mut_with_ctx(&mut self.decoder_inliner, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 3: Running string_array_inliner");
        traverse_mut_with_ctx(&mut self.string_array_inliner, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 3: Running dispatcher_inliner");
        traverse_mut_with_ctx(&mut self.dispatcher_inliner, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 3: Inlined {} decoder calls, {} string-array accesses, {} dispatcher calls",
            self.decoder_inliner.rewrites(),
            self.string_array_inliner.rewrites(),
            self.dispatcher_inliner.rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 3: DONE");
    }

    fn run_proxy_and_call_simplifiers<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 4: SemanticBuilder for call_proxy");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut call_proxy_collector = CallProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 4: Running call_proxy_collector");
        traverse_mut_with_ctx(&mut call_proxy_collector, program, &mut ctx);
        let call_proxies = call_proxy_collector.get_single_use_proxies();
        let found = call_proxies.len();
        let rewrites = if call_proxies.is_empty() {
            0
        } else {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = CallProxyInliner::new(call_proxies);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            inliner.rewrites()
        };
        eprintln!("[DEOBFUSCATE] Phase 4: Found {found} call proxies, {rewrites} inline rewrites");

        Self::run_operator_proxy(allocator, program);
        Self::run_unary_proxy(allocator, program);
        Self::run_apply_concat_and_call_this(allocator, program);
    }

    fn run_operator_proxy<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 5: SemanticBuilder for operator_proxy");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut op_proxy_collector = OperatorProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 5: Running operator_proxy_collector");
        traverse_mut_with_ctx(&mut op_proxy_collector, program, &mut ctx);
        let op_proxies = op_proxy_collector.get_proxies();
        let found = op_proxies.len();
        let rewrites = if op_proxies.is_empty() {
            0
        } else {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = OperatorProxyInliner::new(op_proxies);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            inliner.rewrites()
        };
        eprintln!("[DEOBFUSCATE] Phase 5: Found {found} operator proxies, {rewrites} inline rewrites");
    }

    fn run_unary_proxy<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 5b: SemanticBuilder for unary_proxy");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut un_proxy_collector = UnaryProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 5b: Running unary_proxy_collector");
        traverse_mut_with_ctx(&mut un_proxy_collector, program, &mut ctx);
        let un_proxies = un_proxy_collector.get_proxies();
        eprintln!("[DEOBFUSCATE] Phase 5b: Found {} unary proxies", un_proxies.len());
        if !un_proxies.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut inliner = UnaryProxyInliner::new(un_proxies);
            traverse_mut_with_ctx(&mut inliner, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 5b: Inlined {} unary-proxy call sites",
                inliner.inlined_count()
            );
        }
    }

    fn run_apply_concat_and_call_this<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 5c: apply/call simplifier");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut apply_call = ApplyCallSimplifier::new();
        traverse_mut_with_ctx(&mut apply_call, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5c: Simplified {} .apply/.call sites",
            apply_call.rewrites()
        );

        eprintln!("[DEOBFUSCATE] Phase 5d: concat canonicaliser + String.fromCharCode folder");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut concat = ConcatCanonicaliser::new();
        traverse_mut_with_ctx(&mut concat, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5d: Canonicalised {} .concat(...) calls",
            concat.rewrites()
        );
        let mut fromcc = FromCharCodeFolder::new();
        traverse_mut_with_ctx(&mut fromcc, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5d: Folded {} String.fromCharCode(...) calls",
            fromcc.folded()
        );

        Self::run_dyn_concat(allocator, program);

        eprintln!("[DEOBFUSCATE] Phase 5e: call-this simplifier");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        let mut call_this = CallThisSimplifier::new();
        traverse_mut_with_ctx(&mut call_this, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5e: Simplified {} .call(this, ...) sites",
            call_this.rewrites()
        );
    }

    fn run_dyn_concat<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
        mem_ck("enter Phase 5d.5 (dyn-concat)");
        eprintln!("[DEOBFUSCATE] Phase 5d.5: dyn-concat resolver (string-var inlining)");
        let mut dyn_resolver = DynConcatResolver::new();
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        traverse_mut_with_ctx(&mut dyn_resolver, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 5d.5: {} rewrites", dyn_resolver.rewrites());
    }

    fn run_expression_cluster<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        mem_ck("enter Phase 6 (simplify cluster)");
        eprintln!("[DEOBFUSCATE] Phase 6: SemanticBuilder for simplification passes");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        eprintln!("[DEOBFUSCATE] Phase 6: Running expression_simplifier");
        traverse_mut_with_ctx(&mut self.expression_simplifier, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: Running constant_folder");
        traverse_mut_with_ctx(&mut self.constant_folder, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: Running algebraic_simplifier");
        traverse_mut_with_ctx(&mut self.algebraic_simplifier, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 6: Simplified {} expressions, folded {} constants, simplified {} logical expressions",
            self.expression_simplifier.rewrites(),
            self.constant_folder.rewrites(),
            self.algebraic_simplifier.rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 6: Running array_coerce_fold");
        traverse_mut_with_ctx(&mut self.array_coerce_fold, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 6: Folded {} array-coercion expressions",
            self.array_coerce_fold.folded()
        );
        eprintln!("[DEOBFUSCATE] Phase 6: Running Bun syntax normalizer");
        let mut bun_syntax = BunSyntaxNormalizer::new();
        traverse_mut_with_ctx(&mut bun_syntax, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 6: Normalized {} typeof-undefined and {} numeric idiom expressions",
            bun_syntax.typeof_rewrites(),
            bun_syntax.numeric_rewrites()
        );
        eprintln!("[DEOBFUSCATE] Phase 6: DONE");

        eprintln!("[DEOBFUSCATE] Phase 6.5: comma-return simplifier");
        traverse_mut_with_ctx(&mut self.comma_return_simplifier, program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 6.5: Simplified {} redundant trailing identifiers",
            self.comma_return_simplifier.simplified()
        );
    }

    fn run_dead_var_and_cff<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        eprintln!("[DEOBFUSCATE] Phase 7: SemanticBuilder for dead_code_eliminator");
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
        eprintln!("[DEOBFUSCATE] Phase 7: Running dead_code_eliminator");
        traverse_mut_with_ctx(&mut self.dead_code_eliminator, program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 7: DONE");

        eprintln!("[DEOBFUSCATE] Phase 8: Running validated dead-var sweep");
        let result = eliminate_dead_vars(allocator, program);
        eprintln!(
            "[DEOBFUSCATE] Phase 8: Removed {} dead declarators ({} candidate symbols, rejected={})",
            result.removed_declarators(),
            result.candidate_symbols(),
            result.was_rejected()
        );

        mem_ck("enter Phase 8.5 (CFF unflatten)");
        eprintln!("[DEOBFUSCATE] Phase 8.5: CFF unflattener");
        let detector = DispatcherDetector::new();
        let dispatchers = detector.detect(program);
        eprintln!("[DEOBFUSCATE] Phase 8.5: Found {} dispatchers", dispatchers.len());
        if !dispatchers.is_empty() {
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let case_bodies = collect_case_bodies(program, &dispatchers, allocator);
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            let mut unflattener = CffUnflattener::new(dispatchers, case_bodies);
            traverse_mut_with_ctx(&mut unflattener, program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 8.5: Inlined {} CFF call sites",
                unflattener.inlined()
            );
        }
    }
}
