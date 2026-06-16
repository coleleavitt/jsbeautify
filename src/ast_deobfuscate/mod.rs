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
pub mod array_coerce_fold;
pub mod array_unpack;
pub mod boolean_literals;
pub mod bun_alphabet;
pub mod bun_cjs_flatten;
pub mod bun_export_renamer;
pub mod bun_local_renamer;
pub mod bun_module_annotator;
pub mod bun_module_handles;
pub mod bun_runtime;
pub mod bun_syntax_normalizer;
pub mod call_proxy;
pub mod call_this_simplifier;
pub mod cff_unflattener;
pub mod class_name_renamer;
pub mod comma_return_simplifier;
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
pub mod dyn_concat_resolver;
pub mod dynamic_property;
pub mod empty_statement_cleanup;
pub mod encrypted_eval;
pub mod esbuild_helper;
pub mod expression_simplify;
pub mod fromcharcode_fold;
pub mod function_inline;
pub mod heuristic_rename;
pub mod if_body_comma_split;
pub mod iife_unwrap;
pub mod json_parse_eval;
pub mod llm_rename;
pub mod lookup_forwarder;
pub mod method_call_forwarder;
pub mod multi_var_split;
pub mod nullish_coalescing_simplifier;
pub mod object_sparsing;
pub mod operator_proxy;
pub mod optional_chaining_normalizer;
pub mod param_heuristic;
pub mod post_annotations;
pub mod rename_guards;
pub mod return_sequence_split;
pub mod self_init_accessor;
pub mod sequence_expression_split;
pub mod short_circuit_to_if;
pub mod sourcemap_rename;
pub mod state;
pub mod strength_reduction;
pub mod string_array_inline;
pub mod string_array_rotation;
pub mod string_array_self_defending;
pub mod switch_true_converter;
pub mod ternary;
pub mod ternary_to_if_else;
pub mod trampoline;
pub mod try_catch;
pub mod unary_proxy;
pub mod unicode_mangling;
pub mod variable_rename;
pub mod void_replacer;
pub mod window_alias_propagator;

// Framework / build-tool recon extractors. Previously authored but never
// declared (compile-dead); wired here so they compile and can be driven by
// the `analyzers::framework_recon` dispatcher (recon, not deobfuscation).
pub mod angular_decompiler;
pub mod angular_metadata;
pub mod angular_route_extractor;
pub mod bundle_diff;
pub mod http_interceptor_extractor;
pub mod launchdarkly_extractor;
pub mod mfe_registry_extractor;
pub mod ngrx_extractor;
pub mod rxjs_annotator;
pub mod transmit_sdk_decoder;
pub mod webpack_module_resolver;
pub mod webpack_runtime_decoder;

pub use self::bun_runtime::{BunHelper, BunHelperMap, BunRuntimeCollector, BunRuntimeRenamer};
pub use akamai::{
    AkamaiDeobfuscator, AkamaiDetector, BooleanArithmeticFolder, EqualityProxyUnwrapper, StackTrackerRemover,
    UndefinedPatternNormalizer,
};
pub use algebraic_simplify::AlgebraicSimplifier;
pub use apply_call_simplifier::ApplyCallSimplifier;
pub use array_coerce_fold::ArrayCoerceFold;
pub use array_unpack::ArrayUnpacker;
pub use boolean_literals::BooleanLiteralConverter;
pub use bun_cjs_flatten::flatten_top_level_cjs_wrapper;
pub use bun_export_renamer::{ExportNameCollector, ExportNameRenamer};
pub use bun_module_annotator::annotate_bun_modules;
pub use bun_module_handles::{BunModuleHandleCollector, BunModuleHandleRenamer};
pub use bun_syntax_normalizer::BunSyntaxNormalizer;
pub use call_proxy::{CallProxyCollector, CallProxyInliner};
pub use call_this_simplifier::CallThisSimplifier;
pub use cff_unflattener::{CffUnflattener, collect_case_bodies};
pub use class_name_renamer::{ClassNameCollector, ClassNameRenamer, DisplayNameCollector};
pub use comma_return_simplifier::CommaReturnSimplifier;
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
    CompoundOp, DoWhileCaseInfo, DoWhileDispatcherInfo, DoWhileDispatcherMap, DoWhileSwitchDetector, StateTransition,
    collect_constants, resolve_compound_transitions,
};
pub use dyn_concat_resolver::DynConcatResolver;
pub use dynamic_property::DynamicPropertyConverter;
pub use empty_statement_cleanup::EmptyStatementCleanup;
pub use esbuild_helper::{EsbuildHelperCollector, EsbuildHelperKind, annotate_esbuild_modules};
pub use expression_simplify::ExpressionSimplifier;
pub use fromcharcode_fold::FromCharCodeFolder;
pub use function_inline::{FunctionCollector, FunctionInliner};
pub use if_body_comma_split::IfBodyCommaSplitter;
pub use iife_unwrap::IifeUnwrap;
pub use json_parse_eval::JsonParseEvaluator;
pub use lookup_forwarder::{LookupForwarderCollector, LookupForwarderInliner};
pub use method_call_forwarder::{MethodCallForwarderCollector, MethodCallForwarderInliner};
pub use multi_var_split::MultiVarSplitter;
pub use nullish_coalescing_simplifier::NullishCoalescingSimplifier;
pub use object_sparsing::ObjectSparsingConsolidator;
pub use operator_proxy::{OperatorProxyCollector, OperatorProxyInliner};
pub use optional_chaining_normalizer::OptionalChainingNormalizer;
pub use param_heuristic::ParamHeuristicRenamer;
pub use post_annotations::{annotate_namespace_modules, annotate_ts_enums, expand_arrow_sequences};
pub use return_sequence_split::ReturnSequenceSplitter;
pub use self_init_accessor::SelfInitAccessorFlattener;
pub use sequence_expression_split::SequenceExpressionSplitter;
pub use short_circuit_to_if::ShortCircuitToIf;
pub use state::DeobfuscateState;
pub use strength_reduction::StrengthReducer;
pub use string_array_inline::StringArrayInliner;
pub use string_array_rotation::StringArrayRotation;
pub use string_array_self_defending::SelfDefendingStringArray;
pub use switch_true_converter::SwitchTrueConverter;
pub use ternary::TernarySimplifier;
pub use ternary_to_if_else::TernaryToIfElse;
pub use trampoline::{TrampolineCollector, TrampolineInliner};
pub use try_catch::TryCatchRemover;
pub use unary_proxy::{UnaryProxyCollector, UnaryProxyInliner};
pub use unicode_mangling::UnicodeNormalizer;
pub use variable_rename::VariableRenamer;
pub use void_replacer::VoidReplacer;
pub use window_alias_propagator::{WindowAliasCollector, WindowAliasPropagator};

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashMap;
use std::sync::OnceLock;
use std::time::Instant;

use crate::Result;

/// Resident set size in `MiB` (Linux). Backs the `JSB_MEM` memory-profiling
/// checkpoints below — `/proc/self/statm` field 2 is resident pages.
#[cfg(target_os = "linux")]
fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()))
        .map_or(0, |pages| pages * 4096 / (1024 * 1024))
}
#[cfg(not(target_os = "linux"))]
fn rss_mb() -> u64 {
    0
}

/// Emit an RSS checkpoint when `JSB_MEM` is set, to localise the pipeline's
/// peak-memory phase. No-op (and no syscall) in normal runs.
fn mem_ck(label: &str) {
    if std::env::var_os("JSB_MEM").is_some() {
        eprintln!("[MEM] {:>5} MB  {label}", rss_mb());
    }
}

static TRACE_DEOB: OnceLock<bool> = OnceLock::new();
static TRACE_AST: OnceLock<bool> = OnceLock::new();

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

pub(crate) fn deob_trace_enabled() -> bool {
    *TRACE_DEOB.get_or_init(|| env_flag("JSBEAUTIFY_TRACE_DEOB") || env_flag("JSB_TRACE_DEOB"))
}

pub(crate) fn ast_trace_enabled() -> bool {
    *TRACE_AST.get_or_init(|| env_flag("JSBEAUTIFY_TRACE_AST") || env_flag("JSB_TRACE_AST"))
}

pub(crate) struct TraceSpan {
    label: &'static str,
    start: Instant,
    start_rss: u64,
    enabled: bool,
}

impl TraceSpan {
    #[must_use]
    pub(crate) fn new(label: &'static str) -> Self {
        let enabled = deob_trace_enabled();
        let start_rss = if enabled { rss_mb() } else { 0 };
        if enabled {
            eprintln!("[TRACE] START {label} rss={start_rss} MB");
        }
        Self {
            label,
            start: Instant::now(),
            start_rss,
            enabled,
        }
    }
}

impl Drop for TraceSpan {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let end_rss = rss_mb();
        let rss_delta = i64::try_from(end_rss).unwrap_or(i64::MAX) - i64::try_from(self.start_rss).unwrap_or(i64::MAX);
        eprintln!(
            "[TRACE] END   {} elapsed={:.3}s rss={} MB delta={:+} MB",
            self.label,
            self.start.elapsed().as_secs_f64(),
            end_rss,
            rss_delta
        );
    }
}

pub struct AstDeobfuscator {
    control_flow_unflattener: ControlFlowUnflattener,
    string_array_rotation: StringArrayRotation,
    self_defending_string_array: SelfDefendingStringArray,
    decoder_inliner: DecoderInliner,
    string_array_inliner: StringArrayInliner,
    dispatcher_inliner: DispatcherInliner,
    constant_folder: ConstantFolder,
    expression_simplifier: ExpressionSimplifier,
    algebraic_simplifier: AlgebraicSimplifier,
    array_coerce_fold: ArrayCoerceFold,
    strength_reducer: StrengthReducer,
    comma_return_simplifier: CommaReturnSimplifier,
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
    /// When set, Phase 12.5 infers readable names for minified locals
    /// (`heuristic_rename`) instead of opaque `v{n}` slots.
    readable_names: bool,
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
    #[allow(clippy::too_many_lines)]
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

        // ── Structural half (Phases 0–12.5) runs in its own arena that is freed
        // before the cosmetic half re-parses. The transform passes accumulate a
        // large volume of dead intermediate AST in oxc's bump arena (which never
        // frees mid-run); codegen-ing at the Phase 12.6 checkpoint, dropping this
        // arena, then re-parsing the (valid) output into a fresh arena roughly
        // halves peak RSS. `bun_helper_map` is the only state that must cross.
        let (intermediate, bun_helper_map) = {
            let allocator = Allocator::default();

            let parse_result = Parser::new(&allocator, &code, source_type).parse();

            // oxc recovers from syntax errors and still yields a usable partial
            // AST. Rather than abort the whole deobfuscation (which left odd or
            // non-JS inputs — e.g. an HTTP response dump — with no output at
            // all), log the diagnostics and continue on the recovered program.
            // The final codegen + re-parse checkpoints below validate the
            // result, and the CLI/pipeline fall back to raw on empty output.
            if !parse_result.errors.is_empty() {
                eprintln!(
                    "[DEOBFUSCATE] {} parse diagnostic(s); continuing on recovered AST (first: {:?})",
                    parse_result.errors.len(),
                    parse_result.errors.first().map(ToString::to_string),
                );
            }

            let mut program = parse_result.program;

            eprintln!("[DEOBFUSCATE] Phase 0.4: Window-alias propagation");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut collector = WindowAliasCollector::new();
                traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
                let aliases = collector.aliases();
                if !aliases.is_empty() {
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.4: Found {} window alias(es): {:?}",
                        aliases.len(),
                        aliases
                    );
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut propagator = WindowAliasPropagator::new(aliases);
                    traverse_mut_with_ctx(&mut propagator, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.4: Propagated {} window-alias accesses",
                        propagator.rewrites()
                    );
                }
            }

            eprintln!("[DEOBFUSCATE] Phase 0.5: Akamai BMP detection");
            let mut akamai = AkamaiDeobfuscator::new();
            let is_akamai = akamai.detect(&program);

            if is_akamai {
                eprintln!("[DEOBFUSCATE] Phase 0.5a: Akamai stack-tracker removal");
                akamai.tracker_remover.detect(&program);
                if akamai.tracker_remover.tracker_names().is_empty() {
                    eprintln!("[DEOBFUSCATE] Phase 0.5a: No pure stack-tracker variable identified");
                } else {
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    traverse_mut_with_ctx(&mut akamai.tracker_remover, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.5a: Removed {} stack-tracker calls for trackers {:?}",
                        akamai.tracker_remover.removed_call_count(),
                        akamai.tracker_remover.tracker_names()
                    );
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

            mem_ck("enter Phase 0.9 (helpers)");
            eprintln!("[DEOBFUSCATE] Phase 0.9: Bun runtime helper detection & rename");
            let bun_helper_map = {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut collector = BunRuntimeCollector::new();
                traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
                let map = collector.into_map();
                if map.is_empty() {
                    eprintln!("[DEOBFUSCATE] Phase 0.9: No Bun helpers detected (not a Bun bundle)");
                } else {
                    eprintln!("[DEOBFUSCATE] Phase 0.9: Detected {} Bun runtime helpers", map.len());
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut renamer = BunRuntimeRenamer::new(map.clone());
                    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.9: Renamed {} declarations, {} references",
                        renamer.renamed_decls(),
                        renamer.renamed_refs()
                    );
                }
                map
            };

            eprintln!("[DEOBFUSCATE] Phase 0.91: Bun CJS wrapper flattening");
            let flattened_statements = flatten_top_level_cjs_wrapper(&mut program, &allocator);
            if flattened_statements > 0 {
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.91: Flattened top-level Bun CJS wrapper ({flattened_statements} statements)"
                );
            } else {
                eprintln!("[DEOBFUSCATE] Phase 0.91: No top-level Bun CJS wrapper flattened");
            }

            eprintln!("[DEOBFUSCATE] Phase 0.95: Bun __export() name extraction & rename");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut collector = ExportNameCollector::new();
                traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
                drop(ctx);
                let mut export_map = collector.into_map();

                // Soundness guard: never rename a binding observable to a direct
                // `eval` or exposed as a native ES export (see `rename_guards`).
                if !export_map.is_empty() {
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut guard = rename_guards::RenameGuardCollector::new();
                    traverse_mut_with_ctx(&mut guard, &mut program, &mut ctx);
                    drop(ctx);
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let guarded = guard.into_guarded(&scoping);
                    export_map.retain(|sym, _| !guarded.contains(sym));
                }

                if export_map.is_empty() {
                    eprintln!("[DEOBFUSCATE] Phase 0.95: No __export() calls found");
                } else {
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.95: Found {} export name mappings",
                        export_map.len()
                    );
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    // Make assigned names legal + unique-per-scope to avoid invalid JS
                    // (reserved words like `instanceof`, duplicate `ZodType`/`Client`).
                    let export_map = bun_export_renamer::make_names_unique(export_map, &scoping);
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut renamer = ExportNameRenamer::new(export_map);
                    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.95: Renamed {} declarations + {} references = {} total",
                        renamer.renamed_decls(),
                        renamer.renamed_refs(),
                        renamer.total_renamed()
                    );
                }
            }

            eprintln!("[DEOBFUSCATE] Phase 0.97: Class name detection (this.name / displayName)");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut class_collector = ClassNameCollector::new();
                traverse_mut_with_ctx(&mut class_collector, &mut program, &mut ctx);
                let mut class_map = class_collector.into_map();

                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut display_collector = DisplayNameCollector::new();
                traverse_mut_with_ctx(&mut display_collector, &mut program, &mut ctx);
                for (k, v) in display_collector.into_map() {
                    class_map.entry(k).or_insert(v);
                }

                if !class_map.is_empty() {
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.97: Found {} class/component names",
                        class_map.len()
                    );
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut renamer = ClassNameRenamer::new(class_map);
                    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
                    eprintln!("[DEOBFUSCATE] Phase 0.97: Renamed {} identifiers", renamer.renamed());
                }
            }

            eprintln!("[DEOBFUSCATE] Phase 0.98: Heuristic parameter renaming");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut param_renamer = ParamHeuristicRenamer::new();
                traverse_mut_with_ctx(&mut param_renamer, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 0.98: Renamed {} parameters",
                    param_renamer.renamed()
                );
            }

            mem_ck("enter Phase 0.99 (module handles)");
            eprintln!("[DEOBFUSCATE] Phase 0.99: Bun module-handle renaming");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut collector = BunModuleHandleCollector::new();
                traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
                let module_handles = collector.into_map();
                if module_handles.is_empty() {
                    eprintln!("[DEOBFUSCATE] Phase 0.99: No Bun module wrapper handles found");
                } else {
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.99: Found {} Bun module wrapper handles",
                        module_handles.len()
                    );
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut renamer = BunModuleHandleRenamer::new(module_handles);
                    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 0.99: Renamed {} declarations, {} references",
                        renamer.renamed_decls(),
                        renamer.renamed_refs()
                    );
                }
            }

            mem_ck("enter Phase 1 (control-flow unflatten)");
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

            eprintln!("[DEOBFUSCATE] Phase 2.5: Self-defending string array (javascript-obfuscator modern)");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
            traverse_mut_with_ctx(&mut self.self_defending_string_array, &mut program, &mut ctx);
            let mut state = ctx.into_state();
            self.self_defending_string_array.finalize(&mut state);
            eprintln!(
                "[DEOBFUSCATE] Phase 2.5: Found {} string arrays, {} decoders",
                state.string_arrays.len(),
                state.decoders.len()
            );

            mem_ck("enter Phase 3 (decoder/string-array)");
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

            mem_ck("enter Phase 5d.5 (dyn-concat fixed-point)");
            eprintln!("[DEOBFUSCATE] Phase 5d.5: dyn-concat resolver (string-var inlining, fixed-point)");
            let mut dyn_resolver = DynConcatResolver::new();
            let mut iter = 0u32;
            loop {
                dyn_resolver.reset_changed();
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                traverse_mut_with_ctx(&mut dyn_resolver, &mut program, &mut ctx);
                iter += 1;
                if !dyn_resolver.changed() || iter >= 8 {
                    break;
                }
            }
            eprintln!(
                "[DEOBFUSCATE] Phase 5d.5: {} rewrites over {} iteration(s)",
                dyn_resolver.rewrites(),
                iter
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

            mem_ck("enter Phase 6 (simplify cluster)");
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
            eprintln!("[DEOBFUSCATE] Phase 6: Running array_coerce_fold");
            traverse_mut_with_ctx(&mut self.array_coerce_fold, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 6: Folded {} array-coercion expressions",
                self.array_coerce_fold.folded()
            );
            eprintln!("[DEOBFUSCATE] Phase 6: Running Bun syntax normalizer");
            let mut bun_syntax = BunSyntaxNormalizer::new();
            traverse_mut_with_ctx(&mut bun_syntax, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 6: Normalized {} typeof-undefined and {} numeric idiom expressions",
                bun_syntax.typeof_rewrites(),
                bun_syntax.numeric_rewrites()
            );
            eprintln!("[DEOBFUSCATE] Phase 6: DONE");

            eprintln!("[DEOBFUSCATE] Phase 6.5: comma-return simplifier");
            traverse_mut_with_ctx(&mut self.comma_return_simplifier, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 6.5: Simplified {} redundant trailing identifiers",
                self.comma_return_simplifier.simplified()
            );

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

            mem_ck("enter Phase 8.5 (CFF unflatten)");
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
            let mut dowhile_dispatchers = dowhile_detector.detect(&program);
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

            mem_ck("enter Phase 9 (function inline)");
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
            let di_constants_i64: FxHashMap<String, i64> = di_constants
                .iter()
                .map(|(k, &v)| (k.clone(), i64::try_from(v).unwrap_or(i64::MAX)))
                .collect();
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

            if !dowhile_dispatchers.is_empty() {
                let own_constants = collect_constants(&program);
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
                let mut cleaner = DoWhileSwitchCleaner::new(dowhile_dispatchers, &program);
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);
                eprintln!("[DEOBFUSCATE] Phase 9.7: Pruned {} dead cases", cleaner.pruned_cases());
            }

            mem_ck("enter Phase 10.5 (dead-var sweep)");
            eprintln!("[DEOBFUSCATE] Phase 10.5: Post-CFF dead-var sweep");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut collector = DeadVarCollector::new();
            traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
            let dead_vars = collector.get_dead_vars();
            if !dead_vars.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut eliminator = DeadVarEliminator::new(dead_vars.clone());
                traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 10.5: Eliminated {} dead variables",
                    dead_vars.len()
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

            eprintln!("[DEOBFUSCATE] Phase 10.7: Post-Phase-10 dead-var sweep");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut collector = DeadVarCollector::new();
            traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
            let dead_vars = collector.get_dead_vars();
            if !dead_vars.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut eliminator = DeadVarEliminator::new(dead_vars.clone());
                traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 10.7: Eliminated {} dead variables",
                    dead_vars.len()
                );
            }

            // Rebuild scoping after try_catch_remover and dead_code_eliminator
            // which create new BlockStatement nodes during traversal.
            mem_ck("enter Phase 11 (unicode/bool/void/sparsing)");
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

            eprintln!("[DEOBFUSCATE] Phase 11.5: Post-Phase-11 dead-var sweep");
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut collector = DeadVarCollector::new();
            traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
            let dead_vars = collector.get_dead_vars();
            if !dead_vars.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut eliminator = DeadVarEliminator::new(dead_vars.clone());
                traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 11.5: Eliminated {} dead variables",
                    dead_vars.len()
                );
            }

            {
                let _trace = TraceSpan::new("Phase 12: variable_renamer");
                eprintln!("[DEOBFUSCATE] Phase 12: SemanticBuilder for variable_renamer");
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                eprintln!("[DEOBFUSCATE] Phase 12: Running variable_renamer");
                traverse_mut_with_ctx(&mut self.variable_renamer, &mut program, &mut ctx);
                eprintln!("[DEOBFUSCATE] Phase 12: DONE");
            }

            mem_ck("enter Phase 12.5 (local slot rename)");
            eprintln!("[DEOBFUSCATE] Phase 12.5: Bun local slot renaming (single-char locals -> v0,v1,...)");
            {
                let _trace = TraceSpan::new("Phase 12.5: Bun local slot renaming");
                // Soundness guard: exclude bindings visible to a direct `eval`
                // and native ES exports — renaming either silently breaks
                // runtime equivalence (see `rename_guards`).
                let guard = {
                    let _trace = TraceSpan::new("Phase 12.5: collect rename guards");
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut guard = rename_guards::RenameGuardCollector::new();
                    traverse_mut_with_ctx(&mut guard, &mut program, &mut ctx);
                    drop(ctx);
                    guard
                };

                let guarded = {
                    let _trace = TraceSpan::new("Phase 12.5: resolve rename guards");
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    guard.into_guarded(&scoping)
                };

                // Readable mode: infer heuristic names (and per-scope free-var
                // names for capture safety) at this same seam so proposals share
                // one SymbolId space with the apply.
                let (desired, free_names) = if self.readable_names {
                    let _trace = TraceSpan::new("Phase 12.5: collect readable-name heuristics");
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut collector = heuristic_rename::HeuristicNameCollector::new();
                    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
                    drop(ctx);
                    collector.into_parts()
                } else {
                    Default::default()
                };

                let mode = if self.readable_names {
                    bun_local_renamer::RenameMode::Readable
                } else {
                    bun_local_renamer::RenameMode::Slot
                };
                let (plan, plan_stats) = {
                    let _trace = TraceSpan::new("Phase 12.5: plan local renames");
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    bun_local_renamer::plan_local_renames_with_stats(&scoping, &guarded, mode, &desired, &free_names)
                };
                if deob_trace_enabled() {
                    eprintln!("[DEOBFUSCATE] Phase 12.5: plan stats {plan_stats:?}");
                }
                if plan.is_empty() {
                    eprintln!("[DEOBFUSCATE] Phase 12.5: No single-char locals to rename");
                } else {
                    let _trace = TraceSpan::new("Phase 12.5: apply local renames");
                    let plan_len = plan.len();
                    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                    let mut renamer = bun_local_renamer::LocalRenamer::new(plan);
                    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
                    eprintln!(
                        "[DEOBFUSCATE] Phase 12.5: Renamed {} single-char local bindings to slot names ({} refs, {} function ids, {} planned, {} guarded)",
                        renamer.renamed(),
                        renamer.renamed_refs(),
                        renamer.renamed_functions(),
                        plan_len,
                        guarded.len()
                    );
                }
            }

            // ── Phase 12.6: structural checkpoint. Codegen the program, then the
            // structural arena is freed at the closing brace below — discarding all
            // dead intermediate AST accumulated by Phases 0–12.5.
            mem_ck("Phase 12.6: structural checkpoint codegen");
            eprintln!("[DEOBFUSCATE] Phase 12.6: structural checkpoint (codegen + arena reset)");
            (Codegen::new().build(&program).code, bun_helper_map)
        };

        // ── Cosmetic-A half (Phases 13–15): the statement-expansion passes
        // (empty-stmt, sequence/return/if-body split, multi-var split) that
        // allocate the most new AST. Runs in its own arena, freed at the Phase
        // 15.6 checkpoint before Cosmetic-B re-parses. `bun_helper_map` stays in
        // the fn scope and is consumed at Phase 20, so nothing needs to cross.
        let intermediate2 = {
            let allocator = Allocator::default();
            let parse_result = Parser::new(&allocator, &intermediate, source_type).parse();
            if !parse_result.errors.is_empty() {
                eprintln!(
                    "[DEOBFUSCATE] Phase 12.6 re-parse FAILED ({} error(s)) — dumping context around the first error:",
                    parse_result.errors.len()
                );
                eprintln!(
                    "{}",
                    checkpoint_error_context("Phase 12.6", &intermediate, &parse_result.errors)
                );
                return Err(crate::BeautifyError::BeautificationFailed(format!(
                    "Phase 12.6 checkpoint re-parse failed: {:?}",
                    parse_result.errors.first()
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
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut collector = DeadVarCollector::new();
            traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
            let dead_vars = collector.get_dead_vars();
            if !dead_vars.is_empty() {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut eliminator = DeadVarEliminator::new(dead_vars.clone());
                traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 13.5: Eliminated {} dead variables",
                    dead_vars.len()
                );
            }

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

            eprintln!("[DEOBFUSCATE] Phase 14.7: If-body comma expression splitting");
            {
                let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
                let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
                let mut if_splitter = IfBodyCommaSplitter::new();
                traverse_mut_with_ctx(&mut if_splitter, &mut program, &mut ctx);
                eprintln!(
                    "[DEOBFUSCATE] Phase 14.7: Split {} if-body comma expressions",
                    if_splitter.split_count()
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

        // ── Cosmetic-B half (Phases 16–21): fresh arena, re-parse Cosmetic-A output.
        let allocator = Allocator::default();
        let parse_result = Parser::new(&allocator, &intermediate2, source_type).parse();
        if !parse_result.errors.is_empty() {
            eprintln!(
                "[DEOBFUSCATE] Phase 15.6 re-parse FAILED ({} error(s)) — dumping context around the first error:",
                parse_result.errors.len()
            );
            eprintln!(
                "{}",
                checkpoint_error_context("Phase 15.6", &intermediate2, &parse_result.errors)
            );
            return Err(crate::BeautifyError::BeautificationFailed(format!(
                "Phase 15.6 checkpoint re-parse failed: {:?}",
                parse_result.errors.first()
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

        eprintln!("[DEOBFUSCATE] Phase 19.5: Final dead-var sweep");
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut collector = DeadVarCollector::new();
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let dead_vars = collector.get_dead_vars();
        if !dead_vars.is_empty() {
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            let mut eliminator = DeadVarEliminator::new(dead_vars.clone());
            traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
            eprintln!(
                "[DEOBFUSCATE] Phase 19.5: Eliminated {} dead variables",
                dead_vars.len()
            );
        }

        mem_ck("before codegen (peak structural)");
        eprintln!("[DEOBFUSCATE] Generating output code");
        let output = Codegen::new().build(&program).code;
        eprintln!("[DEOBFUSCATE] Output generated, {} bytes", output.len());
        // Consistency with the arena-split checkpoints: an AST -> codegen boundary
        // that emits unparseable JS indicates a real defect in a prior pass, so
        // surface it rather than returning invalid output as success.
        if !log_output_validity("Phase 19.6", &output, source_type) {
            return Err(crate::BeautifyError::BeautificationFailed(
                "Phase 19.6 final codegen produced unparseable JavaScript".to_string(),
            ));
        }

        if self.skip_annotations {
            eprintln!("[DEOBFUSCATE] Skipping module annotations (alignment mode)");
            return Ok(output);
        }

        eprintln!("[DEOBFUSCATE] Phase 20: Annotating modules");
        let output = annotate_webpack_modules(&output);
        let output = annotate_esbuild_modules(&output, &esbuild_collector);

        let mut esm_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::Esm))
            .map(|(name, _)| name.clone())
            .collect();
        let mut cjs_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::CommonJs))
            .map(|(name, _)| name.clone())
            .collect();

        // Augment from Bun runtime collector if esbuild didn't find anything
        if esm_helpers.is_empty() {
            for (name, helper) in &bun_helper_map {
                if matches!(helper, BunHelper::Esm) {
                    esm_helpers.push(name.clone());
                }
            }
        }
        if cjs_helpers.is_empty() {
            for (name, helper) in &bun_helper_map {
                if matches!(helper, BunHelper::CommonJs) {
                    cjs_helpers.push(name.clone());
                }
            }
        }
        let output = annotate_bun_modules(&output, &esm_helpers, &cjs_helpers);

        eprintln!("[DEOBFUSCATE] Phase 21: Post-codegen annotations (enums, namespaces, arrows)");
        let (output, enum_count) = annotate_ts_enums(&output);
        let (output, ns_count) = annotate_namespace_modules(&output);
        let (output, arrow_count) = expand_arrow_sequences(&output);
        eprintln!(
            "[DEOBFUSCATE] Phase 21: {enum_count} enum annotations, {ns_count} namespace annotations, {arrow_count} arrow expansions"
        );
        // The Phase 20/21 post-codegen transforms (module annotations, arrow
        // expansion) rewrite the output string, so the final result is held to
        // the same parse-validity bar as the AST checkpoints: invalid JS is a
        // real defect and is surfaced rather than returned as success.
        if !log_output_validity("Phase 21.5", &output, source_type) {
            return Err(crate::BeautifyError::BeautificationFailed(
                "Phase 21.5 post-annotation output is invalid JavaScript".to_string(),
            ));
        }

        Ok(output)
    }
}

/// Builds a human-readable context window around the first parse error of a
/// failed checkpoint re-parse. The byte offset is read from the diagnostic's
/// first labelled span via the typed `LabeledSpan::offset()` accessor (a stable
/// public API), then snapped to char boundaries so a multi-byte codepoint near
/// the edge can never panic the slice. A diagnostic with no label (e.g. the
/// source-size guard) falls back to offset 0.
fn checkpoint_error_context(label: &str, code: &str, errors: &[OxcDiagnostic]) -> String {
    let Some(first) = errors.first() else {
        return format!("[DEOBFUSCATE] {label}: <no errors>");
    };
    let offset: usize = first
        .labels
        .as_ref()
        .and_then(|labels| labels.first())
        .map_or(0, oxc_diagnostics::LabeledSpan::offset)
        .min(code.len());

    // Snap the window to char boundaries (offset may land mid-token but should
    // be on a boundary; the ±window edges might not be).
    let mut lo = offset.saturating_sub(300);
    let mut hi = (offset + 300).min(code.len());
    while lo > 0 && !code.is_char_boundary(lo) {
        lo -= 1;
    }
    while hi < code.len() && !code.is_char_boundary(hi) {
        hi += 1;
    }

    format!(
        "[DEOBFUSCATE] {label}: re-parse error at byte {offset} (output len={len}):\n\
         --- context [{lo}..{hi}] ---\n{window}\n--- end context ---",
        len = code.len(),
        window = &code[lo..hi],
    )
}

/// Logs validity of generated output and returns whether it re-parses cleanly.
///
/// Returns `false` only on a *parse* error (the output is not valid JS);
/// semantic errors are logged but still return `true`, matching the
/// parse-only policy the arena-split checkpoints enforce on their own codegen.
fn log_output_validity(label: &str, code: &str, source_type: SourceType) -> bool {
    let allocator = Allocator::default();
    let parse_result = Parser::new(&allocator, code, source_type).parse();
    if !parse_result.errors.is_empty() {
        eprintln!(
            "[DEOBFUSCATE] {label}: GENERATED OUTPUT IS INVALID JAVASCRIPT ({} parse error(s))",
            parse_result.errors.len()
        );
        for (idx, error) in parse_result.errors.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] {label}: parse error {idx}: {error:?}");
        }
        return false;
    }

    let semantic_result = SemanticBuilder::new().build(&parse_result.program);
    if !semantic_result.errors.is_empty() {
        eprintln!(
            "[DEOBFUSCATE] {label}: GENERATED OUTPUT HAS {} SEMANTIC ERROR(S)",
            semantic_result.errors.len()
        );
        for (idx, error) in semantic_result.errors.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] {label}: semantic error {idx}: {error:?}");
        }
        return true;
    }

    eprintln!("[DEOBFUSCATE] {label}: Generated output re-parse succeeded");
    true
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

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    /// Parse `src` and codegen it back to a string with the same `Codegen`
    /// configuration the deobfuscation checkpoints use.
    fn roundtrip(src: &str) -> String {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, src, SourceType::mjs()).parse().program;
        Codegen::new().build(&program).code
    }

    /// The arena-split checkpoints (Phases 12.6 / 15.6) codegen the program,
    /// free the arena, then re-parse the codegen output into a fresh arena. This
    /// is only a behavioural no-op if oxc's `codegen ∘ parse` is idempotent on
    /// our own output — i.e. re-parsing and re-codegen-ing yields byte-identical
    /// text. Because the oxc dependency tracks upstream `main`, a future bump
    /// could silently break that property (and therefore the "1-vs-2-checkpoint
    /// output is byte-identical" guarantee); this test pins it.
    #[test]
    fn codegen_is_idempotent_across_a_checkpoint_roundtrip() {
        // Constructs the checkpoints must preserve across a parse/codegen cycle:
        // hashbang, directives, exports, template literals, regex, comments.
        let samples = [
            "#!/usr/bin/env node\n\"use strict\";\nconst a = 1 + 2;\nexport { a };\n",
            "function f(x){ return x ? `a${x}b` : /re,gex/g.test(x); }\nf(1);\n",
            "export function g(){ let q = 1; return q; }\nexport const K = 2;\n",
            "class C { #p = 1; m(){ return this.#p; } }\nnew C().m();\n",
            "const o = { a: 1, b: 2, ['c']: 3 };\nfor (const k of Object.keys(o)) o[k]++;\n",
        ];

        for src in samples {
            let once = roundtrip(src);
            let twice = roundtrip(&once);
            assert_eq!(
                once, twice,
                "codegen is not idempotent across a checkpoint round-trip for source:\n{src}\n\
                 first:\n{once}\nsecond:\n{twice}"
            );
        }
    }

    /// `checkpoint_error_context` reads the byte offset from a real parse
    /// diagnostic's labelled span (typed `LabeledSpan::offset()`), so a genuine
    /// syntax error must yield a context window centred on the offending byte —
    /// not the fragile string-scrape fallback of 0.
    #[test]
    fn checkpoint_error_context_points_at_the_real_error_offset() {
        // A class body with a stray token: parse fails at a known position.
        let bad = "const a = 1;\nclass C { @@@ }\nconst b = 2;\n";
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, bad, SourceType::mjs()).parse();
        assert!(!parsed.errors.is_empty(), "sample should fail to parse");

        let expected = parsed.errors[0]
            .labels
            .as_ref()
            .and_then(|l| l.first())
            .map(oxc_diagnostics::LabeledSpan::offset)
            .expect("parse error should carry a labelled span");

        let ctx = checkpoint_error_context("TEST", bad, &parsed.errors);
        assert!(
            ctx.contains(&format!("error at byte {expected}")),
            "context should report the diagnostic's real offset {expected}, got:\n{ctx}"
        );
        // The window around a small input is the whole input, including the
        // offending `@` token; confirm the slice did not panic and is non-empty.
        assert!(
            ctx.contains("class C"),
            "context window should include the error site: {ctx}"
        );
    }

    /// Empty `code` must not panic the boundary-snapping / slice logic.
    #[test]
    fn checkpoint_error_context_handles_empty_code() {
        let allocator = Allocator::default();
        // Produce at least one diagnostic, then format against empty code.
        let parsed = Parser::new(&allocator, "(", SourceType::mjs()).parse();
        assert!(!parsed.errors.is_empty());
        let ctx = checkpoint_error_context("TEST", "", &parsed.errors);
        assert!(
            ctx.contains("output len=0"),
            "should clamp offset/window to empty code: {ctx}"
        );
    }
}
