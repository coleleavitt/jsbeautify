use super::{
    AlgebraicSimplifier, ArrayCoerceFold, ArrayUnpacker, AstDeobfuscator, BeautifyError, BooleanLiteralConverter,
    CommaReturnSimplifier, ConstantFolder, ControlFlowUnflattener, DeadCodeEliminator, DecoderInliner,
    DispatcherInliner, DynamicPropertyConverter, EmptyStatementCleanup, ExpressionSimplifier, IifeUnwrap,
    JsonParseEvaluator, Layout, MultiVarSplitter, NullishCoalescingSimplifier, ObjectSparsingConsolidator,
    OptionalChainingNormalizer, Result, SelfDefendingStringArray, SequenceExpressionSplitter, ShortCircuitToIf,
    StringArrayInliner, StringArrayRotation, SwitchTrueConverter, TernarySimplifier, TernaryToIfElse, TryCatchRemover,
    VariableRenamer, VoidReplacer, encrypted_eval, semantic_baseline,
};
use crate::ParseGoal;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
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
            comma_return_simplifier: CommaReturnSimplifier::new(),
            dead_code_eliminator: DeadCodeEliminator::new(),
            array_unpacker: ArrayUnpacker::new(),
            dynamic_property_converter: DynamicPropertyConverter::new(),
            ternary_simplifier: TernarySimplifier::new(),
            try_catch_remover: TryCatchRemover::new(),
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
            layout: None,
            parse_goal: ParseGoal::Auto,
            isolated: false,
            project: crate::ast_deobfuscate::project::ProjectSeeds::default(),
        }
    }

    /// Name bindings by a cross-file naming plan (see
    /// [`crate::ast_deobfuscate::project`]).
    #[must_use]
    pub fn with_project_seeds(mut self, seeds: crate::ast_deobfuscate::project::ProjectSeeds) -> Self {
        self.project = seeds;
        self
    }

    /// Lay out the final output (indentation, `eol`, brace style, …);
    /// without a layout the output is raw tab-indented codegen text.
    #[must_use]
    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = Some(layout);
        self
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

    #[must_use]
    pub const fn with_parse_goal(mut self, goal: ParseGoal) -> Self {
        self.parse_goal = goal;
        self
    }

    /// Treat a classic script's top-level bindings as private (see
    /// [`crate::Options::isolated`]).
    #[must_use]
    pub const fn with_isolated(mut self, isolated: bool) -> Self {
        self.isolated = isolated;
        self
    }

    /// Deobfuscate `code`, returning the transformed source.
    ///
    /// Runs on a worker whose stack is sized from the input length, so input
    /// of any nesting depth is processed.
    ///
    /// # Errors
    /// Returns an error for a panic in the pipeline, parse/checkpoint failure,
    /// or transform failure.
    pub fn deobfuscate(&mut self, code: &str) -> Result<String> {
        crate::stack::run_on_sized_stack(code.len(), || {
            catch_unwind(AssertUnwindSafe(|| self.deobfuscate_inner(code)))
        })
        .map_err(|payload| BeautifyError::DeobfuscationWorkerPanicked(panic_message(payload.as_ref()).to_string()))?
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

        let (intermediate, runtime, source_type) = self.run_structural_half(&code);
        // Measured against the source type the structural half settled on, so
        // the final checkpoints compare like with like.
        let baseline = semantic_baseline(&code, source_type);
        let intermediate2 = self.run_cosmetic_a(&intermediate, source_type)?;
        self.run_cosmetic_b_and_final_output(&intermediate2, source_type, &runtime, baseline)
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
    fn top_level_return_is_parsed_as_commonjs() {
        let output = AstDeobfuscator::new()
            .deobfuscate("x(); return; y();")
            .expect("CommonJS input with a top-level return must deobfuscate");
        assert!(output.contains("return;"), "got: {output}");
    }

    #[test]
    fn program_that_deobfuscates_to_nothing_returns_empty_output() {
        let output = AstDeobfuscator::new()
            .with_parse_goal(ParseGoal::Module)
            .deobfuscate("var a = 1, b = 1;")
            .expect("an all-dead module is a valid result");
        assert!(output.trim().is_empty(), "got: {output}");
    }

    #[test]
    fn unparseable_input_that_recovers_to_nothing_is_an_error() {
        let error = AstDeobfuscator::new()
            .deobfuscate("HTTP/1.1 200 OK\nContent-Type: x\n\n{}")
            .expect_err("no program was recovered");
        assert!(matches!(error, BeautifyError::BeautificationFailed(_)));
    }

    #[test]
    fn workers_can_run_concurrently() {
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
