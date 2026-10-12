mod annotations;
mod checkpoint;
mod cosmetic;
mod final_output;
mod lifecycle;
mod structural;
mod structural_bun;
mod structural_early;
mod structural_late;
mod structural_late_local;
mod structural_mid;
mod trace;

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashMap;

use crate::beautifier::Layout;
use crate::{BeautifyError, Result};

use super::{
    AkamaiDeobfuscator, AlgebraicSimplifier, ApplyCallSimplifier, ArrayCoerceFold, ArrayUnpacker,
    BooleanLiteralConverter, BunHelper, BunHelperMap, BunModuleHandleCollector, BunModuleHandleRenamer,
    BunRuntimeCollector, BunRuntimeRenamer, BunSyntaxNormalizer, BundleRuntime, Bundler, CallProxyCollector,
    CallProxyInliner, CallThisSimplifier, CffUnflattener, ClassNameCollector, ClassNameRenamer, CommaReturnSimplifier,
    ConcatCanonicaliser, ConstantFolder, ControlFlowUnflattener, DeadCodeEliminator, DecoderInliner, DeobfuscateState,
    DispatchInlinerCollector, DispatchInlinerRewriter, DispatcherDetector, DispatcherInliner, DisplayNameCollector,
    DoWhileSwitchCleaner, DoWhileSwitchDetector, DynConcatResolver, DynamicPropertyConverter, EmptyStatementCleanup,
    ExportNameCollector, ExportNameRenamer, ExpressionSimplifier, FromCharCodeFolder, FunctionCollector,
    FunctionInliner, IifeUnwrap, JsonParseEvaluator, LetToConst, LookupForwarderCollector, LookupForwarderInliner,
    MethodCallForwarderCollector, MethodCallForwarderInliner, MinifierIdiomReverser, MultiVarSplitter,
    NullishCoalescingSimplifier, ObjectSparsingConsolidator, OperatorProxyCollector, OperatorProxyInliner,
    OptionalChainingNormalizer, ParamHeuristicRenamer, ReturnSequenceSplitter, SelfDefendingStringArray,
    SelfInitAccessorFlattener, SequenceExpressionSplitter, StatementSplitNormalizer, StringArrayInliner,
    StringArrayRotation, SwitchTrueConverter, TernarySimplifier, TernaryToIfElse, TrampolineCollector,
    TrampolineInliner, TryCatchRemover, UnaryProxyCollector, UnaryProxyInliner, VariableRenamer, VoidReplacer,
    WindowAliasCollector, WindowAliasPropagator, annotate_namespace_modules, annotate_runtime, annotate_ts_enums, bun,
    collect_case_bodies, collect_constants, eliminate_dead_vars, encrypted_eval, expand_arrow_sequences,
    flatten_top_level_cjs_wrapper, heuristic_rename, rename_guards, resolve_compound_transitions,
};
use annotations::annotate_webpack_modules;
use checkpoint::{
    SemanticBaseline, checkpoint_error_context, log_output_validity, log_parsed_validity, semantic_baseline,
};
use trace::{TraceSpan, mem_ck};

pub use trace::{ast_trace_enabled, deob_trace_enabled};

#[allow(clippy::struct_excessive_bools)]
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
    comma_return_simplifier: CommaReturnSimplifier,
    dead_code_eliminator: DeadCodeEliminator,
    array_unpacker: ArrayUnpacker,
    dynamic_property_converter: DynamicPropertyConverter,
    ternary_simplifier: TernarySimplifier,
    try_catch_remover: TryCatchRemover,
    boolean_literal_converter: BooleanLiteralConverter,
    void_replacer: VoidReplacer,
    object_sparsing_consolidator: ObjectSparsingConsolidator,
    variable_renamer: VariableRenamer,
    empty_statement_cleanup: EmptyStatementCleanup,
    sequence_expression_splitter: SequenceExpressionSplitter,
    multi_var_splitter: MultiVarSplitter,
    ternary_to_if_else: TernaryToIfElse,
    switch_true_converter: SwitchTrueConverter,
    iife_unwrap: IifeUnwrap,
    json_parse_evaluator: JsonParseEvaluator,
    optional_chaining_normalizer: OptionalChainingNormalizer,
    nullish_coalescing_simplifier: NullishCoalescingSimplifier,
    skip_annotations: bool,
    /// When set, Phase 12.5 infers readable names for minified locals
    /// (`heuristic_rename`) instead of opaque `v{n}` slots.
    readable_names: bool,
    layout: Option<Layout>,
    parse_goal: crate::ParseGoal,
    isolated: bool,
    /// This file's part of a project naming plan (`--project`); empty for a
    /// single file.
    project: super::project::ProjectSeeds,
    keep_fn_names: bool,
    short_export_names: bool,
    window_alias_es5: bool,
}
