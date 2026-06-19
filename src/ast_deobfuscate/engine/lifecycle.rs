use super::{
    AlgebraicSimplifier, ArrayCoerceFold, ArrayUnpacker, AstDeobfuscator, BooleanLiteralConverter,
    CommaReturnSimplifier, ConstantFolder, ControlFlowUnflattener, DeadCodeEliminator, DecoderInliner,
    DispatcherInliner, DynamicPropertyConverter, EmptyStatementCleanup, ExpressionSimplifier, IifeUnwrap,
    JsonParseEvaluator, MultiVarSplitter, NullishCoalescingSimplifier, ObjectSparsingConsolidator,
    OptionalChainingNormalizer, Result, SelfDefendingStringArray, SequenceExpressionSplitter, ShortCircuitToIf,
    SourceType, StrengthReducer, StringArrayInliner, StringArrayRotation, SwitchTrueConverter, TernarySimplifier,
    TernaryToIfElse, TryCatchRemover, UnicodeNormalizer, VariableRenamer, VoidReplacer, encrypted_eval,
};

impl AstDeobfuscator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            control_flow_unflattener: ControlFlowUnflattener::new(),
            string_array_rotation: StringArrayRotation::new(),
            self_defending_string_array: SelfDefendingStringArray::new(),
            decoder_inliner: DecoderInliner::new(),
            string_array_inliner: StringArrayInliner::new(),
            dispatcher_inliner: DispatcherInliner::new(),
            constant_folder: ConstantFolder::new(),
            expression_simplifier: ExpressionSimplifier::new(),
            algebraic_simplifier: AlgebraicSimplifier::new(),
            array_coerce_fold: ArrayCoerceFold::new(),
            strength_reducer: StrengthReducer::new(),
            comma_return_simplifier: CommaReturnSimplifier::new(),
            dead_code_eliminator: DeadCodeEliminator::new(),
            array_unpacker: ArrayUnpacker::new(),
            dynamic_property_converter: DynamicPropertyConverter::new(),
            ternary_simplifier: TernarySimplifier::new(),
            try_catch_remover: TryCatchRemover::new(),
            unicode_normalizer: UnicodeNormalizer::new(),
            boolean_literal_converter: BooleanLiteralConverter::new(),
            void_replacer: VoidReplacer::new(),
            object_sparsing_consolidator: ObjectSparsingConsolidator::new(),
            variable_renamer: VariableRenamer::new(),
            empty_statement_cleanup: EmptyStatementCleanup::new(),
            sequence_expression_splitter: SequenceExpressionSplitter::new(),
            multi_var_splitter: MultiVarSplitter::new(),
            ternary_to_if_else: TernaryToIfElse::new(),
            switch_true_converter: SwitchTrueConverter::new(),
            short_circuit_to_if: ShortCircuitToIf::new(),
            iife_unwrap: IifeUnwrap::new(),
            json_parse_evaluator: JsonParseEvaluator::new(),
            optional_chaining_normalizer: OptionalChainingNormalizer::new(),
            nullish_coalescing_simplifier: NullishCoalescingSimplifier::new(),
            skip_annotations: false,
            readable_names: false,
        }
    }

    #[must_use]
    pub const fn with_skip_annotations(mut self, skip: bool) -> Self {
        self.skip_annotations = skip;
        self
    }

    /// Enable readable heuristic naming of minified locals (Phase 12.5).
    #[must_use]
    pub const fn with_readable_names(mut self, readable: bool) -> Self {
        self.readable_names = readable;
        self
    }

    /// Deobfuscate `code`, returning the transformed source.
    ///
    /// CWE-674 hardening: parse + transform recurse in input nesting depth, and
    /// a Rust stack overflow *aborts the process* (it is not a catchable panic).
    /// So we run the work on a worker thread with a large stack — raising the
    /// overflow threshold ~20x for every deep-recursion shape (bracket nesting,
    /// `a+a+...` operator chains, `a.b.b...` member chains) so hostile-but-
    /// bounded input is processed instead of crashing — and reject input whose
    /// raw bracket nesting is pathological up front as a final backstop.
    ///
    /// # Errors
    /// Returns an error if the operation fails.
    ///
    /// # Panics
    /// Panics only if the OS refuses to spawn the worker thread (an environment
    /// failure, not an input condition).
    pub fn deobfuscate(&mut self, code: &str) -> Result<String> {
        // Backstop for the extreme bracket-nesting case (which overflows the
        // parser itself, before any pass runs). Far above any real or normally-
        // obfuscated source, so it never rejects legitimate input.
        const MAX_BRACKET_DEPTH: usize = 100_000;
        const WORKER_STACK_BYTES: usize = 256 * 1024 * 1024;

        let depth = Self::max_bracket_depth(code);
        if depth > MAX_BRACKET_DEPTH {
            eprintln!(
                "[DEOBFUSCATE] input bracket-nesting depth {depth} exceeds cap {MAX_BRACKET_DEPTH}; returning input unchanged (suspected DoS)"
            );
            return Ok(code.to_string());
        }

        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(WORKER_STACK_BYTES)
                .spawn_scoped(scope, || self.deobfuscate_inner(code))
                .expect("spawn deobfuscate worker thread")
                .join()
                .unwrap_or_else(|_| {
                    eprintln!("[DEOBFUSCATE] worker thread aborted; returning input unchanged");
                    Ok(code.to_string())
                })
        })
    }

    /// Maximum bracket-nesting depth of the raw source — a cheap `O(n)` scan used
    /// as a `DoS` safety valve. Not string/comment aware, which is fine: the cap
    /// it feeds is far above any real code.
    fn max_bracket_depth(s: &str) -> usize {
        let (mut depth, mut max) = (0usize, 0usize);
        for b in s.bytes() {
            match b {
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    max = max.max(depth);
                }
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        max
    }
    /// # Errors
    /// Returns an error if the operation fails.
    fn deobfuscate_inner(&mut self, code: &str) -> Result<String> {
        let code = encrypted_eval::decrypt_encrypted_evals(code).map_or_else(
            || code.to_string(),
            |decrypted| {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0: Decrypted encrypted eval payload ({} bytes)",
                    decrypted.len()
                );
                decrypted
            },
        );

        let source_type = SourceType::mjs();
        let (intermediate, bun_helper_map) = self.run_structural_half(&code, source_type);
        let intermediate2 = self.run_cosmetic_a(&intermediate, source_type)?;
        self.run_cosmetic_b_and_final_output(&intermediate2, source_type, &bun_helper_map)
    }
}
impl Default for AstDeobfuscator {
    fn default() -> Self {
        Self::new()
    }
}
