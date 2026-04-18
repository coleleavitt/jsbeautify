//! AST-based deobfuscation using Oxc
//!
//! This module implements all deobfuscation transformations as AST traversals
//! using the Oxc parser and traversal infrastructure. This ensures:
//! - Valid JavaScript output (no syntax errors from token manipulation)
//! - Access to full Oxc optimization suite
//! - Better performance (single parse/codegen cycle)
//! - More maintainable code following `oxc_minifier` patterns

pub mod akamai;
pub mod algebraic_simplify;
pub mod apply_call_simplifier;
pub mod array_unpack;
pub mod boolean_literals;
pub mod bun_alphabet;
pub mod bun_module_annotator;
pub mod call_proxy;
pub mod call_this_simplifier;
pub mod cff_unflattener;
pub mod concat_canonicaliser;
pub mod constant_folding;
pub mod control_flow_unflatten;
pub mod dead_code;
pub mod dead_var_elimination;
pub mod decoder_inline;
pub mod deterministic_rename;
pub mod dispatch_inliner;
pub mod dispatcher_detector;
pub mod dispatcher_inline;
pub mod dowhile_switch_cleaner;
pub mod dowhile_switch_detector;
pub mod dynamic_property;
pub mod empty_statement_cleanup;
pub mod encrypted_eval;
pub mod esbuild_helper;
pub mod expression_simplify;
pub mod fromcharcode_fold;
pub mod function_inline;
pub mod iife_unwrap;
pub mod json_parse_eval;
pub mod lookup_forwarder;
pub mod method_call_forwarder;
pub mod multi_var_split;
pub mod nullish_coalescing_simplifier;
pub mod object_sparsing;
pub mod operator_proxy;
pub mod optional_chaining_normalizer;
pub mod self_init_accessor;
pub mod sequence_expression_split;
pub mod short_circuit_to_if;
pub mod state;
pub mod strength_reduction;
pub mod string_array_inline;
pub mod string_array_rotation;
pub mod switch_true_converter;
pub mod ternary;
pub mod ternary_to_if_else;
pub mod trampoline;
pub mod try_catch;
pub mod unary_proxy;
pub mod unicode_mangling;
pub mod variable_rename;
pub mod void_replacer;

pub use akamai::{
    AkamaiDeobfuscator, AkamaiDetector, BooleanArithmeticFolder, EqualityProxyUnwrapper, StackTrackerRemover,
    UndefinedPatternNormalizer,
};
pub use algebraic_simplify::AlgebraicSimplifier;
pub use apply_call_simplifier::ApplyCallSimplifier;
pub use array_unpack::ArrayUnpacker;
pub use boolean_literals::BooleanLiteralConverter;
pub use bun_module_annotator::annotate_bun_modules;
pub use call_proxy::{CallProxyCollector, CallProxyInliner};
pub use call_this_simplifier::CallThisSimplifier;
pub use cff_unflattener::{CffUnflattener, collect_case_bodies};
pub use concat_canonicaliser::ConcatCanonicaliser;
pub use constant_folding::ConstantFolder;
pub use control_flow_unflatten::ControlFlowUnflattener;
pub use dead_code::DeadCodeEliminator;
pub use dead_var_elimination::{DeadVarCollector, DeadVarEliminator};
pub use decoder_inline::DecoderInliner;
pub use deterministic_rename::DeterministicRenamer;
pub use dispatch_inliner::{DispatchInlinerCollector, DispatchInlinerRewriter};
pub use dispatcher_detector::{CaseInfo, DispatcherDetector, DispatcherInfo, DispatcherMap};
pub use dispatcher_inline::DispatcherInliner;
pub use dowhile_switch_cleaner::DoWhileSwitchCleaner;
pub use dowhile_switch_detector::{
    DoWhileCaseInfo, DoWhileDispatcherInfo, DoWhileDispatcherMap, DoWhileSwitchDetector, StateTransition,
};
pub use dynamic_property::DynamicPropertyConverter;
pub use empty_statement_cleanup::EmptyStatementCleanup;
pub use esbuild_helper::{EsbuildHelperCollector, EsbuildHelperKind, annotate_esbuild_modules};
pub use expression_simplify::ExpressionSimplifier;
pub use fromcharcode_fold::FromCharCodeFolder;
pub use function_inline::{FunctionCollector, FunctionInliner};
pub use iife_unwrap::IifeUnwrap;
pub use json_parse_eval::JsonParseEvaluator;
pub use lookup_forwarder::{LookupForwarderCollector, LookupForwarderInliner};
pub use method_call_forwarder::{MethodCallForwarderCollector, MethodCallForwarderInliner};
pub use multi_var_split::MultiVarSplitter;
pub use nullish_coalescing_simplifier::NullishCoalescingSimplifier;
pub use object_sparsing::ObjectSparsingConsolidator;
pub use operator_proxy::{OperatorProxyCollector, OperatorProxyInliner};
pub use optional_chaining_normalizer::OptionalChainingNormalizer;
pub use self_init_accessor::SelfInitAccessorFlattener;
pub use sequence_expression_split::SequenceExpressionSplitter;
pub use short_circuit_to_if::ShortCircuitToIf;
pub use state::DeobfuscateState;
pub use strength_reduction::StrengthReducer;
pub use string_array_inline::StringArrayInliner;
pub use string_array_rotation::StringArrayRotation;
pub use switch_true_converter::SwitchTrueConverter;
pub use ternary::TernarySimplifier;
pub use ternary_to_if_else::TernaryToIfElse;
pub use trampoline::{TrampolineCollector, TrampolineInliner};
pub use try_catch::TryCatchRemover;
pub use unary_proxy::{UnaryProxyCollector, UnaryProxyInliner};
pub use unicode_mangling::UnicodeNormalizer;
pub use variable_rename::VariableRenamer;
pub use void_replacer::VoidReplacer;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

use crate::Result;

pub struct AstDeobfuscator {
    control_flow_unflattener: ControlFlowUnflattener,
    string_array_rotation: StringArrayRotation,
    decoder_inliner: DecoderInliner,
    string_array_inliner: StringArrayInliner,
    dispatcher_inliner: DispatcherInliner,
    constant_folder: ConstantFolder,
    expression_simplifier: ExpressionSimplifier,
    algebraic_simplifier: AlgebraicSimplifier,
    strength_reducer: StrengthReducer,
    dead_code_eliminator: DeadCodeEliminator,
    array_unpacker: ArrayUnpacker,
    dynamic_property_converter: DynamicPropertyConverter,
    ternary_simplifier: TernarySimplifier,
    try_catch_remover: TryCatchRemover,
    unicode_normalizer: UnicodeNormalizer,
    boolean_literal_converter: BooleanLiteralConverter,
    void_replacer: VoidReplacer,
    object_sparsing_consolidator: ObjectSparsingConsolidator,
    variable_renamer: VariableRenamer,
    deterministic_renamer: DeterministicRenamer,
    empty_statement_cleanup: EmptyStatementCleanup,
    sequence_expression_splitter: SequenceExpressionSplitter,
    multi_var_splitter: MultiVarSplitter,
    ternary_to_if_else: TernaryToIfElse,
    switch_true_converter: SwitchTrueConverter,
    short_circuit_to_if: ShortCircuitToIf,
    iife_unwrap: IifeUnwrap,
    json_parse_evaluator: JsonParseEvaluator,
    optional_chaining_normalizer: OptionalChainingNormalizer,
    nullish_coalescing_simplifier: NullishCoalescingSimplifier,
    skip_annotations: bool,
}

impl AstDeobfuscator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            control_flow_unflattener: ControlFlowUnflattener::new(),
            string_array_rotation: StringArrayRotation::new(),
            decoder_inliner: DecoderInliner::new(),
            string_array_inliner: StringArrayInliner::new(),
            dispatcher_inliner: DispatcherInliner::new(),
            constant_folder: ConstantFolder::new(),
            expression_simplifier: ExpressionSimplifier::new(),
            algebraic_simplifier: AlgebraicSimplifier::new(),
            strength_reducer: StrengthReducer::new(),
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
            deterministic_renamer: DeterministicRenamer::new(),
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
        }
    }

    #[must_use]
    pub fn with_skip_annotations(mut self, skip: bool) -> Self {
        self.skip_annotations = skip;
        self
    }

    /// # Errors
    /// Returns an error if the operation fails.
    #[allow(clippy::too_many_lines)]
    pub fn deobfuscate(&mut self, code: &str) -> Result<String> {
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

        let allocator = Allocator::default();
        let source_type = SourceType::mjs();

        let parse_result = Parser::new(&allocator, &code, source_type).parse();

        if !parse_result.errors.is_empty() {
            return Err(crate::BeautifyError::BeautificationFailed(format!(
                "Parse failed: {:?}",
                parse_result.errors.first()
            )));
        }

        let mut program = parse_result.program;

        eprintln!("[DEOBFUSCATE] Phase 0.5: Akamai BMP detection");
        let mut akamai = AkamaiDeobfuscator::new();
        let is_akamai = akamai.detect(&program);

        if is_akamai {
            eprintln!("[DEOBFUSCATE] Phase 0.5a: Akamai stack-tracker removal");
            akamai.tracker_remover.detect(&program);
            if !akamai.tracker_remover.tracker_names().is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                traverse_mut_with_ctx(&mut akamai.tracker_remover, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.5a: Removed {} stack-tracker calls for trackers {:?}",
                    akamai.tracker_remover.removed_call_count(),
                    akamai.tracker_remover.tracker_names()
                );
            } else {
                eprintln!("[DEOBFUSCATE] Phase 0.5a: No pure stack-tracker variable identified");
            }

            eprintln!("[DEOBFUSCATE] Phase 0.5b: Akamai undefined-pattern + boolean-arithmetic folding");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut akamai.undef_normalizer, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5b: Replaced {} [][[]] patterns with undefined",
                akamai.undef_normalizer.replaced_count()
            );
            traverse_mut_with_ctx(&mut akamai.bool_folder, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5b: Folded {} boolean-arithmetic expressions",
                akamai.bool_folder.folded_count()
            );

            eprintln!("[DEOBFUSCATE] Phase 0.5c: Akamai equality-proxy unwrapping");
            akamai.eq_unwrapper.collect(&program);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5c: Found {} equality-proxy functions",
                akamai.eq_unwrapper.proxies().len()
            );
            if !akamai.eq_unwrapper.proxies().is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                traverse_mut_with_ctx(&mut akamai.eq_unwrapper, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.5c: Unwrapped {} equality-proxy call sites",
                    akamai.eq_unwrapper.unwrapped_count()
                );
            }

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

            eprintln!("[DEOBFUSCATE] Phase 0.5d: Akamai self-init-accessor flattener");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut accessor_flattener = SelfInitAccessorFlattener::new();
            traverse_mut_with_ctx(&mut accessor_flattener, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5d: Flattened {} self-init accessors",
                accessor_flattener.rewritten()
            );

            eprintln!("[DEOBFUSCATE] Phase 0.5e: Akamai lookup-forwarder inliner");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut fwd_collector = LookupForwarderCollector::new();
            traverse_mut_with_ctx(&mut fwd_collector, &mut program, &mut ctx);
            let forwarders = fwd_collector.forwarders();
            eprintln!("[DEOBFUSCATE] Phase 0.5e: Found {} lookup forwarders", forwarders.len());
            if !forwarders.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut inliner = LookupForwarderInliner::new(forwarders);
                traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.5e: Inlined {} lookup-forwarder call sites",
                    inliner.inlined()
                );
            }

            eprintln!("[DEOBFUSCATE] Phase 0.5e2: Akamai 2-arg method-call forwarder inliner");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut mc_collector = MethodCallForwarderCollector::new();
            traverse_mut_with_ctx(&mut mc_collector, &mut program, &mut ctx);
            let mc_forwarders = mc_collector.into_forwarders();
            eprintln!(
                "[DEOBFUSCATE] Phase 0.5e2: Found {} method-call forwarders",
                mc_forwarders.len()
            );
            if !mc_forwarders.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut inliner = MethodCallForwarderInliner::new(mc_forwarders);
                traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.5e2: Inlined {} method-call-forwarder call sites",
                    inliner.inlined()
                );
            }

            eprintln!("[DEOBFUSCATE] Phase 0.5f: Akamai trampoline inliner");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut tramp_collector = TrampolineCollector::new();
            traverse_mut_with_ctx(&mut tramp_collector, &mut program, &mut ctx);
            let trampolines = tramp_collector.into_trampolines();
            eprintln!("[DEOBFUSCATE] Phase 0.5f: Found {} trampolines", trampolines.len());
            if !trampolines.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut inliner = TrampolineInliner::new(trampolines);
                traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.5f: Inlined {} trampoline call sites",
                    inliner.inlined()
                );
            }
        }

        eprintln!("[DEOBFUSCATE] Phase 1: SemanticBuilder for control_flow_unflattener");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);

        eprintln!("[DEOBFUSCATE] Phase 1: Running control_flow_unflattener");
        traverse_mut_with_ctx(&mut self.control_flow_unflattener, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 1: control_flow_unflattener DONE");

        eprintln!("[DEOBFUSCATE] Phase 2: SemanticBuilder for string_array_rotation");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);

        eprintln!("[DEOBFUSCATE] Phase 2: Running string_array_rotation");
        traverse_mut_with_ctx(&mut self.string_array_rotation, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 2: string_array_rotation DONE");

        let mut state = ctx.into_state();
        self.string_array_rotation.finalize(&mut state);

        eprintln!("[DEOBFUSCATE] Phase 3: SemanticBuilder for decoder/string_array/dispatcher");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        eprintln!("[DEOBFUSCATE] Phase 3: Running decoder_inliner");
        traverse_mut_with_ctx(&mut self.decoder_inliner, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 3: Running string_array_inliner");
        traverse_mut_with_ctx(&mut self.string_array_inliner, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 3: Running dispatcher_inliner");
        traverse_mut_with_ctx(&mut self.dispatcher_inliner, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 3: DONE");

        eprintln!("[DEOBFUSCATE] Phase 4: SemanticBuilder for call_proxy");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut call_proxy_collector = CallProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 4: Running call_proxy_collector");
        traverse_mut_with_ctx(&mut call_proxy_collector, &mut program, &mut ctx);
        let call_proxies = call_proxy_collector.get_single_use_proxies();
        if !call_proxies.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut inliner = CallProxyInliner::new(call_proxies);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        eprintln!("[DEOBFUSCATE] Phase 5: SemanticBuilder for operator_proxy");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut op_proxy_collector = OperatorProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 5: Running operator_proxy_collector");
        traverse_mut_with_ctx(&mut op_proxy_collector, &mut program, &mut ctx);
        let op_proxies = op_proxy_collector.get_proxies();
        if !op_proxies.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut inliner = OperatorProxyInliner::new(op_proxies);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        eprintln!("[DEOBFUSCATE] Phase 5b: SemanticBuilder for unary_proxy");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut un_proxy_collector = UnaryProxyCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 5b: Running unary_proxy_collector");
        traverse_mut_with_ctx(&mut un_proxy_collector, &mut program, &mut ctx);
        let un_proxies = un_proxy_collector.get_proxies();
        eprintln!("[DEOBFUSCATE] Phase 5b: Found {} unary proxies", un_proxies.len());
        if !un_proxies.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut inliner = UnaryProxyInliner::new(un_proxies);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 5b: Inlined {} unary-proxy call sites",
                inliner.inlined_count()
            );
        }

        eprintln!("[DEOBFUSCATE] Phase 5c: apply/call simplifier");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut apply_call = ApplyCallSimplifier::new();
        traverse_mut_with_ctx(&mut apply_call, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5c: Simplified {} .apply/.call sites",
            apply_call.rewrites()
        );

        eprintln!("[DEOBFUSCATE] Phase 5d: concat canonicaliser + String.fromCharCode folder");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut concat = ConcatCanonicaliser::new();
        traverse_mut_with_ctx(&mut concat, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5d: Canonicalised {} .concat(...) calls",
            concat.rewrites()
        );
        let mut fromcc = FromCharCodeFolder::new();
        traverse_mut_with_ctx(&mut fromcc, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5d: Folded {} String.fromCharCode(...) calls",
            fromcc.folded()
        );

        eprintln!("[DEOBFUSCATE] Phase 5e: call-this simplifier");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut call_this = CallThisSimplifier::new();
        traverse_mut_with_ctx(&mut call_this, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 5e: Simplified {} .call(this, ...) sites",
            call_this.rewrites()
        );

        eprintln!("[DEOBFUSCATE] Phase 6: SemanticBuilder for simplification passes");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 6: Running expression_simplifier");
        traverse_mut_with_ctx(&mut self.expression_simplifier, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: Running constant_folder");
        traverse_mut_with_ctx(&mut self.constant_folder, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: Running algebraic_simplifier");
        traverse_mut_with_ctx(&mut self.algebraic_simplifier, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: Running strength_reducer");
        traverse_mut_with_ctx(&mut self.strength_reducer, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 6: DONE");

        eprintln!("[DEOBFUSCATE] Phase 7: SemanticBuilder for dead_code_eliminator");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 7: Running dead_code_eliminator");
        traverse_mut_with_ctx(&mut self.dead_code_eliminator, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 7: DONE");

        eprintln!("[DEOBFUSCATE] Phase 8: SemanticBuilder for dead_var");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut collector = DeadVarCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 8: Running dead_var_collector");
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let dead_vars = collector.get_dead_vars();
        if !dead_vars.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut eliminator = DeadVarEliminator::new(dead_vars);
            traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
        }

        eprintln!("[DEOBFUSCATE] Phase 8.5: CFF unflattener");
        let detector = DispatcherDetector::new();
        let dispatchers = detector.detect(&program);
        eprintln!("[DEOBFUSCATE] Phase 8.5: Found {} dispatchers", dispatchers.len());
        if !dispatchers.is_empty() {
            let case_bodies = collect_case_bodies(&program, &dispatchers, &allocator);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut unflattener = CffUnflattener::new(dispatchers, case_bodies);
            traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 8.5: Inlined {} CFF call sites",
                unflattener.inlined()
            );
        }

        eprintln!("[DEOBFUSCATE] Phase 8.7: do-while-switch detector");
        let dowhile_detector = DoWhileSwitchDetector::new();
        let dowhile_dispatchers = dowhile_detector.detect(&program);
        for (name, info) in &dowhile_dispatchers {
            eprintln!(
                "[DOWHILE] found {}({}, {}) with {} cases, exit_sentinel={}",
                name,
                info.state_param,
                info.args_param,
                info.cases.len(),
                info.exit_sentinel
            );
        }

        if !dowhile_dispatchers.is_empty() {
            eprintln!("[DEOBFUSCATE] Phase 8.8: do-while-switch dead-case pruner");
            let mut cleaner = DoWhileSwitchCleaner::new(dowhile_dispatchers, &program);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);
            eprintln!("[DEOBFUSCATE] Phase 8.8: Pruned {} dead cases", cleaner.pruned_cases());
        }

        eprintln!("[DEOBFUSCATE] Phase 9: SemanticBuilder for function_inline");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut func_collector = FunctionCollector::new();
        eprintln!("[DEOBFUSCATE] Phase 9: Running function_collector");
        traverse_mut_with_ctx(&mut func_collector, &mut program, &mut ctx);
        let single_use = func_collector.get_single_use_functions();
        if !single_use.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut inliner = FunctionInliner::new(single_use);
            traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
        }

        eprintln!("[DEOBFUSCATE] Phase 9.5: dispatch inliner (string-array-factory resolution)");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut di_collector = DispatchInlinerCollector::new();
        traverse_mut_with_ctx(&mut di_collector, &mut program, &mut ctx);
        let (di_factories, di_constants) = di_collector.into_maps();
        eprintln!(
            "[DEOBFUSCATE] Phase 9.5: Found {} string-array factories, {} index constants",
            di_factories.len(),
            di_constants.len()
        );
        if !di_factories.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut di_rewriter = DispatchInlinerRewriter::new(di_factories, di_constants);
            traverse_mut_with_ctx(&mut di_rewriter, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 9.5: Inlined {} dispatch call sites",
                di_rewriter.inlined()
            );
        }

        eprintln!("[DEOBFUSCATE] Phase 10: SemanticBuilder for array/dynamic/ternary/try_catch");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 10: Running array_unpacker");
        traverse_mut_with_ctx(&mut self.array_unpacker, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running dynamic_property_converter");
        traverse_mut_with_ctx(&mut self.dynamic_property_converter, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running ternary_simplifier");
        traverse_mut_with_ctx(&mut self.ternary_simplifier, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: Running try_catch_remover");
        traverse_mut_with_ctx(&mut self.try_catch_remover, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 10: DONE");

        // Rebuild scoping after try_catch_remover and dead_code_eliminator
        // which create new BlockStatement nodes during traversal.
        eprintln!("[DEOBFUSCATE] Phase 11: SemanticBuilder for unicode/boolean/void/object_sparsing");
        let semantic_result = SemanticBuilder::new().build(&program);
        eprintln!(
            "[DEOBFUSCATE] Phase 11: SemanticBuilder completed, errors: {}",
            semantic_result.errors.len()
        );
        for (i, err) in semantic_result.errors.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] Phase 11: Semantic error {i}: {err:?}");
        }
        let scoping = semantic_result.semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 11: Running unicode_normalizer");
        traverse_mut_with_ctx(&mut self.unicode_normalizer, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: unicode_normalizer DONE");
        eprintln!("[DEOBFUSCATE] Phase 11: Running boolean_literal_converter");
        traverse_mut_with_ctx(&mut self.boolean_literal_converter, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: Running void_replacer");
        traverse_mut_with_ctx(&mut self.void_replacer, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: Running object_sparsing_consolidator");
        traverse_mut_with_ctx(&mut self.object_sparsing_consolidator, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 11: DONE");

        eprintln!("[DEOBFUSCATE] Phase 12: SemanticBuilder for variable_renamer");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 12: Running variable_renamer");
        traverse_mut_with_ctx(&mut self.variable_renamer, &mut program, &mut ctx);
        eprintln!("[DEOBFUSCATE] Phase 12: DONE");

        eprintln!("[DEOBFUSCATE] Phase 13: SemanticBuilder for empty_statement_cleanup");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 13: Running empty_statement_cleanup");
        traverse_mut_with_ctx(&mut self.empty_statement_cleanup, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 13: Removed {} empty statements",
            self.empty_statement_cleanup.removed_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 14: SemanticBuilder for sequence_expression_split");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 14: Running sequence_expression_splitter");
        traverse_mut_with_ctx(&mut self.sequence_expression_splitter, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 14: Split {} sequence expressions",
            self.sequence_expression_splitter.split_count()
        );

        eprintln!("[DEOBFUSCATE] Phase 15: SemanticBuilder for multi_var_split");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 15: Running multi_var_splitter");
        traverse_mut_with_ctx(&mut self.multi_var_splitter, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 15: Split {} multi-var declarations",
            self.multi_var_splitter.split_count()
        );

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
        eprintln!("[DEOBFUSCATE] Phase 17: SemanticBuilder for short_circuit_to_if");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        eprintln!("[DEOBFUSCATE] Phase 17: Running short_circuit_to_if");
        traverse_mut_with_ctx(&mut self.short_circuit_to_if, &mut program, &mut ctx);
        eprintln!(
            "[DEOBFUSCATE] Phase 17: Converted {} short-circuit expressions to if statements",
            self.short_circuit_to_if.converted_count()
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

        eprintln!("[DEOBFUSCATE] Phase 19: Detecting esbuild helpers");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut esbuild_collector = EsbuildHelperCollector::new();
        traverse_mut_with_ctx(&mut esbuild_collector, &mut program, &mut ctx);

        eprintln!("[DEOBFUSCATE] Generating output code");
        let output = Codegen::new().build(&program).code;
        eprintln!("[DEOBFUSCATE] Output generated, {} bytes", output.len());

        if self.skip_annotations {
            eprintln!("[DEOBFUSCATE] Skipping module annotations (alignment mode)");
            return Ok(output);
        }

        eprintln!("[DEOBFUSCATE] Phase 20: Annotating modules");
        let output = annotate_webpack_modules(&output);
        let output = annotate_esbuild_modules(&output, &esbuild_collector);

        let esm_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::Esm))
            .map(|(name, _)| name.clone())
            .collect();
        let cjs_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::CommonJs))
            .map(|(name, _)| name.clone())
            .collect();
        let output = annotate_bun_modules(&output, &esm_helpers, &cjs_helpers);

        Ok(output)
    }
}

fn annotate_webpack_modules(code: &str) -> String {
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0u32;

    for line in code.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("var ")
            && let Some(name_end) = rest.find(" = v(() =>")
        {
            let module_name = &rest[..name_end];
            if !module_name.is_empty() && module_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                result.push_str("// ═══════════════════════════════════════\n");
                result.push_str("// Webpack Module: ");
                result.push_str(module_name);
                result.push('\n');
                result.push_str("// ═══════════════════════════════════════\n");
                count = count.wrapping_add(1);
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if count > 0 {
        eprintln!("[DEOBFUSCATE] Phase 19: Annotated {count} webpack modules");
    }

    result
}

impl Default for AstDeobfuscator {
    fn default() -> Self {
        Self::new()
    }
}
