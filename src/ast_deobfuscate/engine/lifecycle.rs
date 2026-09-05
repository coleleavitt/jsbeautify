use super::{
    AlgebraicSimplifier, ArrayCoerceFold, ArrayUnpacker, AstDeobfuscator, BeautifyError, BooleanLiteralConverter,
    CommaReturnSimplifier, ConstantFolder, ControlFlowUnflattener, DeadCodeEliminator, DecoderInliner,
    DispatcherInliner, DynamicPropertyConverter, EmptyStatementCleanup, ExpressionSimplifier, IifeUnwrap,
    JsonParseEvaluator, MultiVarSplitter, NullishCoalescingSimplifier, ObjectSparsingConsolidator,
    OptionalChainingNormalizer, Result, SelfDefendingStringArray, SequenceExpressionSplitter, ShortCircuitToIf,
    SourceType, StrengthReducer, StringArrayInliner, StringArrayRotation, SwitchTrueConverter, TernarySimplifier,
    TernaryToIfElse, TryCatchRemover, UnicodeNormalizer, VariableRenamer, VoidReplacer, encrypted_eval,
};

const MAX_BRACKET_DEPTH: usize = 100_000;
const MIN_WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_WORKER_STACK_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKER_STACK_BYTES: usize = 64 * 1024 * 1024;

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn run_worker<T, F>(stack_bytes: usize, work: F) -> Result<T>
where
    T: Send,
    F: FnOnce() -> Result<T> + Send,
{
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("ast-deobfuscator".into())
            .stack_size(stack_bytes)
            .spawn_scoped(scope, work)
            .map_err(|error| BeautifyError::DeobfuscationWorkerSpawnFailed(error.to_string()))?;

        worker.join().map_err(|payload| {
            BeautifyError::DeobfuscationWorkerPanicked(panic_message(payload.as_ref()).to_string())
        })?
    })
}

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
            worker_stack_bytes: DEFAULT_WORKER_STACK_BYTES,
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

    /// Set the deobfuscation worker stack size in bytes.
    ///
    /// Values are clamped to a safe range so one call cannot reserve an
    /// unbounded amount of virtual address space.
    #[must_use]
    pub fn with_worker_stack_bytes(mut self, bytes: usize) -> Self {
        self.worker_stack_bytes = bytes.clamp(MIN_WORKER_STACK_BYTES, MAX_WORKER_STACK_BYTES);
        self
    }

    /// Deobfuscate `code`, returning the transformed source.
    ///
    /// Parse and transform work runs on a bounded-stack worker because some
    /// parser paths recurse in proportion to input nesting depth.
    ///
    /// # Errors
    /// Returns an error for pathological nesting, worker-spawn failure, worker
    /// panic, parse/checkpoint failure, or transform failure.
    pub fn deobfuscate(&mut self, code: &str) -> Result<String> {
        let depth = Self::max_bracket_depth(code);
        if depth > MAX_BRACKET_DEPTH {
            return Err(BeautifyError::InvalidInput(format!(
                "input bracket-nesting depth {depth} exceeds cap {MAX_BRACKET_DEPTH}"
            )));
        }

        let stack_bytes = self.worker_stack_bytes;
        run_worker(stack_bytes, || self.deobfuscate_inner(code))
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
        let (intermediate, bun_helper_map, source_type) = self.run_structural_half(&code, source_type);
        let intermediate2 = self.run_cosmetic_a(&intermediate, source_type)?;
        self.run_cosmetic_b_and_final_output(&intermediate2, source_type, &bun_helper_map)
    }
}
impl Default for AstDeobfuscator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_panic_is_a_distinct_error() {
        let error = run_worker(DEFAULT_WORKER_STACK_BYTES, || -> Result<()> {
            panic!("worker panic sentinel")
        })
        .expect_err("worker panic must not become successful unchanged input");

        assert!(matches!(
            error,
            BeautifyError::DeobfuscationWorkerPanicked(message)
                if message == "worker panic sentinel"
        ));
    }

    #[test]
    fn worker_spawn_failure_is_a_distinct_error() {
        let error = run_worker(usize::MAX, || Ok(())).expect_err("an impossible stack reservation must fail to spawn");

        assert!(matches!(error, BeautifyError::DeobfuscationWorkerSpawnFailed(_)));
    }

    #[test]
    fn worker_preserves_inner_errors() {
        let error = run_worker(DEFAULT_WORKER_STACK_BYTES, || -> Result<()> {
            Err(BeautifyError::InvalidInput("parse sentinel".into()))
        })
        .expect_err("inner errors must remain errors");

        assert!(matches!(
            error,
            BeautifyError::InvalidInput(message) if message == "parse sentinel"
        ));
    }

    #[test]
    fn configured_worker_stack_is_bounded() {
        assert_eq!(
            AstDeobfuscator::new().with_worker_stack_bytes(0).worker_stack_bytes,
            MIN_WORKER_STACK_BYTES
        );
        assert_eq!(
            AstDeobfuscator::new()
                .with_worker_stack_bytes(usize::MAX)
                .worker_stack_bytes,
            MAX_WORKER_STACK_BYTES
        );
        assert_eq!(AstDeobfuscator::new().worker_stack_bytes, DEFAULT_WORKER_STACK_BYTES);
    }

    #[test]
    fn pathological_nesting_returns_an_error() {
        let code = "(".repeat(MAX_BRACKET_DEPTH + 1);
        let error = AstDeobfuscator::new()
            .deobfuscate(&code)
            .expect_err("nesting cap must fail closed");

        assert!(matches!(error, BeautifyError::InvalidInput(_)));
    }

    #[test]
    fn bounded_workers_can_run_concurrently() {
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let output = AstDeobfuscator::new()
                            .deobfuscate("globalThis.answer = 6 * 7;")
                            .expect("concurrent deobfuscation should succeed");
                        assert!(output.contains("answer"));
                    })
                })
                .collect();

            for worker in workers {
                worker.join().expect("caller thread should not panic");
            }
        });
    }
}
