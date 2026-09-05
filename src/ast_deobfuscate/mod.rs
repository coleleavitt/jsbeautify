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
pub(crate) mod pass;
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
pub use dead_var_elimination::{DeadVarCollector, DeadVarEliminator, DeadVarSweepResult, eliminate_dead_vars};
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

mod engine;

pub use engine::AstDeobfuscator;
pub(crate) use engine::{ast_trace_enabled, deob_trace_enabled};
