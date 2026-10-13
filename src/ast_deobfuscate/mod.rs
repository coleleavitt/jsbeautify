//! AST-based deobfuscation using Oxc
//!
//! This module implements all deobfuscation transformations as AST traversals
//! using the Oxc parser and traversal infrastructure. This ensures:
//! - Valid JavaScript output (no syntax errors from token manipulation)
//! - Access to full Oxc optimization suite
//! - Better performance (single parse/codegen cycle)
//! - More maintainable code following `oxc_minifier` patterns
//!
//! The supported transformation facade is [`AstDeobfuscator`]. Individual
//! traversal passes are implementation details and are not part of the public API.
//! Source-map and LLM naming utilities remain public because the command-line
//! application uses them directly.
//!
//! ```compile_fail
//! // Pass implementations are intentionally not exposed to downstream crates.
//! use jsbeautify::ast_deobfuscate::ConstantFolder;
//! ```

// Private module hubs intentionally re-export selected sibling items for the
// engine. Once those hubs are crate-private, some compatibility re-exports are
// not used in every target. Dormant recon and optional pass modules are kept
// compiled for their unit tests and planned pipeline integration; their module-
// scoped dead-code allowances do not hide diagnostics elsewhere in the crate.
#![allow(unused_imports)]

/// `eprintln!` that only prints under `JSB_TRACE_DEOB=1`. Use it for
/// per-node and per-occurrence pass logging; the engine keeps one summary
/// line per phase on stderr in normal runs.
macro_rules! deob_trace {
    ($($arg:tt)*) => {
        if crate::ast_deobfuscate::deob_trace_enabled() {
            eprintln!($($arg)*);
        }
    };
}

pub(crate) mod akamai;
#[allow(dead_code)]
pub(crate) mod algebraic_simplify;
pub(crate) mod angular;
pub(crate) mod apply_call_simplifier;
pub(crate) mod array_coerce_fold;
#[allow(dead_code)]
pub(crate) mod array_unpack;
#[allow(dead_code)]
pub(crate) mod boolean_literals;
pub(crate) mod bun;
#[allow(dead_code)]
pub(crate) mod call_proxy;
pub(crate) mod call_sites;
pub(crate) mod call_this_simplifier;
pub(crate) mod cff_unflattener;
#[allow(dead_code)]
pub(crate) mod class_name_renamer;
pub(crate) mod comma_return_simplifier;
pub(crate) mod concat_canonicaliser;
#[allow(dead_code)]
pub(crate) mod constant_folding;
#[allow(dead_code)]
pub(crate) mod control_flow_unflatten;
#[allow(dead_code)]
pub(crate) mod dead_code;
#[allow(dead_code)]
pub(crate) mod dead_var_elimination;
#[allow(dead_code)]
pub(crate) mod decoder_inline;
pub(crate) mod default_export;
#[allow(dead_code)]
pub(crate) mod deterministic_rename;
pub(crate) mod dispatch_inliner;
#[allow(dead_code)]
pub(crate) mod dispatcher_detector;
#[allow(dead_code)]
pub(crate) mod dispatcher_inline;
pub(crate) mod dowhile_switch_cleaner;
pub(crate) mod dowhile_switch_detector;
pub(crate) mod dyn_concat_resolver;
#[allow(dead_code)]
pub(crate) mod dynamic_property;
pub(crate) mod empty_statement_cleanup;
pub(crate) mod encrypted_eval;
#[allow(dead_code)]
pub(crate) mod expression_simplify;
pub(crate) mod fromcharcode_fold;
pub(crate) mod function_context;
#[allow(dead_code)]
pub(crate) mod function_inline;
pub(crate) mod heuristic_rename;
pub(crate) mod iife_unwrap;
pub(crate) mod json_parse_eval;
pub(crate) mod let_to_const;
pub mod llm_rename;
pub(crate) mod lookup_forwarder;
pub(crate) mod method_call_forwarder;
pub(crate) mod minifier_idioms;
pub(crate) mod multi_var_split;
pub(crate) mod nullish_coalescing_simplifier;
#[allow(dead_code)]
pub(crate) mod object_sparsing;
#[allow(dead_code)]
pub(crate) mod operator_proxy;
pub(crate) mod optional_chaining_normalizer;
pub(crate) mod param_heuristic;
pub(crate) mod pass;
pub(crate) mod post_annotations;
pub mod project;
pub(crate) mod rename_guards;
pub(crate) mod rename_planner;
pub(crate) mod return_sequence_split;
pub(crate) mod self_init_accessor;
pub(crate) mod semantic;
pub(crate) mod sequence_expression_split;
pub(crate) mod short_circuit_to_if;
pub mod sourcemap_rename;
pub(crate) mod state;
pub(crate) mod statement_split;
#[allow(dead_code)]
pub(crate) mod string_array_inline;
pub(crate) mod string_array_rotation;
pub(crate) mod string_array_self_defending;
pub(crate) mod switch_true_converter;
#[allow(dead_code)]
pub(crate) mod ternary;
pub(crate) mod ternary_to_if_else;
pub(crate) mod trampoline;
#[allow(dead_code)]
pub(crate) mod try_catch;
pub(crate) mod turbopack;
pub(crate) mod unary_proxy;
#[allow(dead_code)]
pub(crate) mod variable_rename;
#[allow(dead_code)]
pub(crate) mod void_replacer;
pub(crate) mod webpack;
pub(crate) mod window_alias_propagator;

pub use akamai::{
    AkamaiDeobfuscator, AkamaiDetector, BooleanArithmeticFolder, EqualityProxyUnwrapper, StackTrackerRemover,
    UndefinedPatternNormalizer,
};
pub(crate) use algebraic_simplify::AlgebraicSimplifier;
pub(crate) use apply_call_simplifier::ApplyCallSimplifier;
pub(crate) use array_coerce_fold::ArrayCoerceFold;
pub(crate) use array_unpack::ArrayUnpacker;
pub(crate) use boolean_literals::BooleanLiteralConverter;
pub(crate) use bun::{
    BunHelper, BunHelperMap, BunModuleHandleCollector, BunModuleHandleRenamer, BunRuntimeCollector, BunRuntimeRenamer,
    BunSyntaxNormalizer, BundleRuntime, Bundler, ExportNameCollector, ExportNameRenamer, annotate_runtime,
    flatten_top_level_cjs_wrapper,
};
pub(crate) use call_proxy::{CallProxyCollector, CallProxyInliner};
pub(crate) use call_this_simplifier::CallThisSimplifier;
pub(crate) use cff_unflattener::{CffUnflattener, collect_case_bodies};
pub(crate) use class_name_renamer::{ClassNameCollector, ClassNameRenamer, DisplayNameCollector};
pub(crate) use comma_return_simplifier::CommaReturnSimplifier;
pub(crate) use concat_canonicaliser::ConcatCanonicaliser;
pub(crate) use constant_folding::ConstantFolder;
pub(crate) use control_flow_unflatten::ControlFlowUnflattener;
pub(crate) use dead_code::DeadCodeEliminator;
pub(crate) use dead_var_elimination::{DeadVarCollector, DeadVarEliminator, DeadVarSweepResult, eliminate_dead_vars};
pub(crate) use decoder_inline::DecoderInliner;
pub(crate) use deterministic_rename::DeterministicRenamer;
pub(crate) use dispatch_inliner::{DispatchInlinerCollector, DispatchInlinerRewriter};
pub(crate) use dispatcher_detector::{CaseInfo, DispatcherDetector, DispatcherInfo, DispatcherMap};
pub(crate) use dispatcher_inline::DispatcherInliner;
pub(crate) use dowhile_switch_cleaner::DoWhileSwitchCleaner;
pub use dowhile_switch_detector::{
    CompoundOp, DoWhileCaseInfo, DoWhileDispatcherInfo, DoWhileDispatcherMap, DoWhileSwitchDetector, StateTransition,
    collect_constants, resolve_compound_transitions,
};
pub(crate) use dyn_concat_resolver::DynConcatResolver;
pub(crate) use dynamic_property::DynamicPropertyConverter;
pub(crate) use empty_statement_cleanup::EmptyStatementCleanup;
pub(crate) use expression_simplify::ExpressionSimplifier;
pub(crate) use fromcharcode_fold::FromCharCodeFolder;
pub(crate) use function_inline::{FunctionCollector, FunctionInliner};
pub(crate) use iife_unwrap::IifeUnwrap;
pub(crate) use json_parse_eval::JsonParseEvaluator;
pub(crate) use let_to_const::LetToConst;
pub(crate) use lookup_forwarder::{LookupForwarderCollector, LookupForwarderInliner};
pub(crate) use method_call_forwarder::{MethodCallForwarderCollector, MethodCallForwarderInliner};
pub(crate) use minifier_idioms::MinifierIdiomReverser;
pub(crate) use multi_var_split::MultiVarSplitter;
pub(crate) use nullish_coalescing_simplifier::NullishCoalescingSimplifier;
pub(crate) use object_sparsing::ObjectSparsingConsolidator;
pub(crate) use operator_proxy::{OperatorProxyCollector, OperatorProxyInliner};
pub(crate) use optional_chaining_normalizer::OptionalChainingNormalizer;
pub(crate) use param_heuristic::ParamHeuristicRenamer;
pub(crate) use post_annotations::{annotate_namespace_modules, annotate_ts_enums, expand_arrow_sequences};
pub(crate) use return_sequence_split::ReturnSequenceSplitter;
pub(crate) use self_init_accessor::SelfInitAccessorFlattener;
pub(crate) use sequence_expression_split::SequenceExpressionSplitter;
pub(crate) use state::DeobfuscateState;
pub(crate) use statement_split::StatementSplitNormalizer;
pub(crate) use string_array_inline::StringArrayInliner;
pub(crate) use string_array_rotation::StringArrayRotation;
pub(crate) use string_array_self_defending::SelfDefendingStringArray;
pub(crate) use switch_true_converter::SwitchTrueConverter;
pub(crate) use ternary::TernarySimplifier;
pub(crate) use ternary_to_if_else::TernaryToIfElse;
pub(crate) use trampoline::{TrampolineCollector, TrampolineInliner};
pub(crate) use try_catch::TryCatchRemover;
pub(crate) use unary_proxy::{UnaryProxyCollector, UnaryProxyInliner};
pub(crate) use variable_rename::VariableRenamer;
pub(crate) use void_replacer::VoidReplacer;
pub(crate) use window_alias_propagator::{WindowAliasCollector, WindowAliasPropagator};

mod engine;

pub use engine::AstDeobfuscator;
pub(crate) use engine::{ast_trace_enabled, deob_trace_enabled};
