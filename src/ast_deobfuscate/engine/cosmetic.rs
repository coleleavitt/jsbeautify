use super::{
    Allocator, AstDeobfuscator, BodyBlockWrapper, BunRuntimeCollector, BundleRuntime, Codegen, DeobfuscateState,
    LetToConst, MinifierIdiomReverser, Parser, Program, Result, ReturnSequenceSplitter, ReusableTraverseCtx,
    SemanticBaseline, SemanticBuilder, SourceType, checkpoint_error_context, eliminate_dead_vars, mem_ck,
    traverse_mut_with_ctx,
};

const STATEMENT_SPLIT_MAX_ROUNDS: usize = 8;

impl AstDeobfuscator {
    pub(super) fn run_cosmetic_a(&mut self, intermediate: &str, source_type: SourceType) -> Result<String> {
        let intermediate2 = {
            let allocator = Allocator::default();
            let parse_result = Parser::new(&allocator, intermediate, source_type).parse();
            if !parse_result.diagnostics.is_empty() {
                eprintln!(
                    "[DEOBFUSCATE] Phase 12.6 re-parse FAILED ({} error(s)) — dumping context around the first error:",
                    parse_result.diagnostics.len()
                );
                eprintln!(
                    "{}",
                    checkpoint_error_context("Phase 12.6", intermediate, &parse_result.diagnostics)
                );
                return Err(crate::BeautifyError::BeautificationFailed(format!(
                    "Phase 12.6 checkpoint re-parse failed: {:?}",
                    parse_result.diagnostics.first()
                )));
            }
            let mut program = parse_result.program;
            mem_ck("cosmetic-A start (after re-parse)");

            eprintln!("[DEOBFUSCATE] Phase 13: SemanticBuilder for empty_statement_cleanup");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            eprintln!("[DEOBFUSCATE] Phase 13: Running empty_statement_cleanup");
            traverse_mut_with_ctx(&mut self.empty_statement_cleanup, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 13: Removed {} empty statements",
                self.empty_statement_cleanup.removed_count()
            );

            eprintln!("[DEOBFUSCATE] Phase 13.5: Post-Phase-13 dead-var sweep");
            let result = eliminate_dead_vars(&allocator, &mut program);
            eprintln!(
                "[DEOBFUSCATE] Phase 13.5: Removed {} dead declarators ({} candidate symbols, rejected={})",
                result.removed_declarators(),
                result.candidate_symbols(),
                result.was_rejected()
            );

            mem_ck("enter Phase 14 (sequence split)");
            eprintln!("[DEOBFUSCATE] Phase 14: SemanticBuilder for sequence_expression_split");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            eprintln!("[DEOBFUSCATE] Phase 14: Running sequence_expression_splitter");
            traverse_mut_with_ctx(&mut self.sequence_expression_splitter, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 14: Split {} sequence expressions",
                self.sequence_expression_splitter.split_count()
            );

            eprintln!("[DEOBFUSCATE] Phase 14.5: Return-sequence splitting");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut return_splitter = ReturnSequenceSplitter::new();
                traverse_mut_with_ctx(&mut return_splitter, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 14.5: Split {} return sequences",
                    return_splitter.split_count()
                );
            }

            mem_ck("enter Phase 15 (multi-var split)");
            eprintln!("[DEOBFUSCATE] Phase 15: SemanticBuilder for multi_var_split");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            eprintln!("[DEOBFUSCATE] Phase 15: Running multi_var_splitter");
            traverse_mut_with_ctx(&mut self.multi_var_splitter, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 15: Split {} multi-var declarations",
                self.multi_var_splitter.split_count()
            );

            // ── Phase 15.6: second checkpoint — codegen Cosmetic-A; this arena is
            // freed at the closing brace, reclaiming the split-expansion AST.
            mem_ck("Phase 15.6: cosmetic-A checkpoint codegen");
            eprintln!("[DEOBFUSCATE] Phase 15.6: cosmetic-A checkpoint (codegen + arena reset)");
            Codegen::new().build(&program).code
        };
        Ok(intermediate2)
    }

    pub(super) fn run_cosmetic_b_and_final_output(
        &mut self,
        intermediate2: &str,
        source_type: SourceType,
        runtime: &BundleRuntime,
        baseline: SemanticBaseline,
    ) -> Result<String> {
        // ── Cosmetic-B half (Phases 16–21): fresh arena, re-parse Cosmetic-A output.
        let allocator = Allocator::default();
        let parse_result = Parser::new(&allocator, intermediate2, source_type).parse();
        if !parse_result.diagnostics.is_empty() {
            eprintln!(
                "[DEOBFUSCATE] Phase 15.6 re-parse FAILED ({} error(s)) — dumping context around the first error:",
                parse_result.diagnostics.len()
            );
            eprintln!(
                "{}",
                checkpoint_error_context("Phase 15.6", intermediate2, &parse_result.diagnostics)
            );
            return Err(crate::BeautifyError::BeautificationFailed(format!(
                "Phase 15.6 checkpoint re-parse failed: {:?}",
                parse_result.diagnostics.first()
            )));
        }
        let mut program = parse_result.program;
        mem_ck("cosmetic-B start (after re-parse)");

        mem_ck("enter Phase 16 (ternary-to-if)");
        eprintln!("[DEOBFUSCATE] Phase 16: SemanticBuilder for ternary_to_if_else");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 16: Running ternary_to_if_else");
        traverse_mut_with_ctx(&mut self.ternary_to_if_else, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 16: Converted {} ternary expressions to if/else",
            self.ternary_to_if_else.converted_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 16.5: SemanticBuilder for switch_true_converter");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 16.5: Running switch_true_converter");
        traverse_mut_with_ctx(&mut self.switch_true_converter, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 16.5: Converted {} switch(true) statements to if/else",
            self.switch_true_converter.converted_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 16.7: SemanticBuilder for json_parse_evaluator");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 16.7: Running json_parse_evaluator");
        traverse_mut_with_ctx(&mut self.json_parse_evaluator, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 16.7: Evaluated {} JSON.parse() calls",
            self.json_parse_evaluator.evaluated_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 16.9: SemanticBuilder for optional_chaining_normalizer");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 16.9: Running optional_chaining_normalizer");
        traverse_mut_with_ctx(&mut self.optional_chaining_normalizer, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 16.9: Detected {} optional chaining expressions",
            self.optional_chaining_normalizer.detected_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 17.1: SemanticBuilder for nullish_coalescing_simplifier");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 17.1: Running nullish_coalescing_simplifier");
        traverse_mut_with_ctx(&mut self.nullish_coalescing_simplifier, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 17.1: Detected {} nullish coalescing chains",
            self.nullish_coalescing_simplifier.detected_count()
        );
        mem_ck("enter Phase 17 (statement-split fixpoint)");
        eprintln!("[DEOBFUSCATE] Phase 17: statement-split fixpoint");
        self.run_statement_split_fixpoint(&allocator, &mut program);

        eprintln!("[DEOBFUSCATE] Phase 17.5: SemanticBuilder for minifier idiom reversal");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut idiom_reverser = MinifierIdiomReverser::new();
        traverse_mut_with_ctx(&mut idiom_reverser, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 17.5: Reversed {} minifier idioms",
            idiom_reverser.rewritten_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 17.6: SemanticBuilder for let_to_const");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut let_to_const = LetToConst::new();
        traverse_mut_with_ctx(&mut let_to_const, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 17.6: Converted {} let declarations to const",
            let_to_const.converted_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 18: SemanticBuilder for iife_unwrap");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 18: Running iife_unwrap");
        traverse_mut_with_ctx(&mut self.iife_unwrap, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 18: Unwrapped {} IIFEs",
            self.iife_unwrap.unwrapped_count()
        );

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut helper_collector = BunRuntimeCollector::new();
        traverse_mut_with_ctx(&mut helper_collector, &mut program, &mut ctx);
        let helpers = helper_collector.into_map();
        eprintln!(
            "[DEOBFUSCATE] Phase 19: Collected {} runtime helpers for module annotation",
            helpers.len()
        );

        eprintln!("[DEOBFUSCATE] Phase 19.5: Final dead-var sweep");
        let result = eliminate_dead_vars(&allocator, &mut program);
        eprintln!(
            "[DEOBFUSCATE] Phase 19.5: Removed {} dead declarators ({} candidate symbols, rejected={})",
            result.removed_declarators(),
            result.candidate_symbols(),
            result.was_rejected()
        );

        self.generate_final_output(&program, source_type, &helpers, runtime.bundler, baseline)
    }

    /// Each statement-split pass rewrites only the statement lists it exits, so
    /// a statement one pass creates (the `if` body `short_circuit_to_if` builds
    /// from `x && (y ? f() : g())`) is only revisited by another round. The
    /// passes only create scopes, never read bindings, so a round shares one
    /// semantic build.
    fn run_statement_split_fixpoint<'a>(&mut self, allocator: &'a Allocator, program: &mut Program<'a>) {
        let mut return_splitter = ReturnSequenceSplitter::new();
        let mut body_wrapper = BodyBlockWrapper::new();
        let total = |this: &Self, return_splitter: &ReturnSequenceSplitter, body_wrapper: &BodyBlockWrapper| {
            this.short_circuit_to_if.converted_count()
                + this.ternary_to_if_else.converted_count()
                + this.sequence_expression_splitter.split_count()
                + return_splitter.split_count()
                + body_wrapper.wrapped_count()
        };
        for round in 1..=STATEMENT_SPLIT_MAX_ROUNDS {
            let before = total(self, &return_splitter, &body_wrapper);
            let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
            traverse_mut_with_ctx(&mut body_wrapper, program, &mut ctx);
            traverse_mut_with_ctx(&mut self.short_circuit_to_if, program, &mut ctx);
            traverse_mut_with_ctx(&mut self.ternary_to_if_else, program, &mut ctx);
            traverse_mut_with_ctx(&mut self.sequence_expression_splitter, program, &mut ctx);
            traverse_mut_with_ctx(&mut return_splitter, program, &mut ctx);
            let rewrites = total(self, &return_splitter, &body_wrapper) - before;
            eprintln!("[DEOBFUSCATE] Phase 17: round {round}: {rewrites} statement rewrites");
            if rewrites == 0 {
                break;
            }
        }
    }
}
