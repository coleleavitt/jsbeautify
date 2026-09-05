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

use crate::Result;

use super::{
    AkamaiDeobfuscator, AlgebraicSimplifier, ApplyCallSimplifier, ArrayCoerceFold, ArrayUnpacker,
    BooleanLiteralConverter, BunHelper, BunHelperMap, BunModuleHandleCollector, BunModuleHandleRenamer,
    BunRuntimeCollector, BunRuntimeRenamer, BunSyntaxNormalizer, CallProxyCollector, CallProxyInliner,
    CallThisSimplifier, CffUnflattener, ClassNameCollector, ClassNameRenamer, CommaReturnSimplifier,
    ConcatCanonicaliser, ConstantFolder, ControlFlowUnflattener, DeadCodeEliminator, DecoderInliner, DeobfuscateState,
    DispatchInlinerCollector, DispatchInlinerRewriter, DispatcherDetector, DispatcherInliner, DisplayNameCollector,
    DoWhileSwitchCleaner, DoWhileSwitchDetector, DynConcatResolver, DynamicPropertyConverter, EmptyStatementCleanup,
    EsbuildHelperCollector, EsbuildHelperKind, ExportNameCollector, ExportNameRenamer, ExpressionSimplifier,
    FromCharCodeFolder, FunctionCollector, FunctionInliner, IfBodyCommaSplitter, IifeUnwrap, JsonParseEvaluator,
    LookupForwarderCollector, LookupForwarderInliner, MethodCallForwarderCollector, MethodCallForwarderInliner,
    MultiVarSplitter, NullishCoalescingSimplifier, ObjectSparsingConsolidator, OperatorProxyCollector,
    OperatorProxyInliner, OptionalChainingNormalizer, ParamHeuristicRenamer, ReturnSequenceSplitter,
    SelfDefendingStringArray, SelfInitAccessorFlattener, SequenceExpressionSplitter, ShortCircuitToIf, StrengthReducer,
    StringArrayInliner, StringArrayRotation, SwitchTrueConverter, TernarySimplifier, TernaryToIfElse,
    TrampolineCollector, TrampolineInliner, TryCatchRemover, UnaryProxyCollector, UnaryProxyInliner, UnicodeNormalizer,
    VariableRenamer, VoidReplacer, WindowAliasCollector, WindowAliasPropagator, annotate_bun_modules,
    annotate_esbuild_modules, annotate_namespace_modules, annotate_ts_enums, bun_export_renamer, bun_local_renamer,
    collect_case_bodies, collect_constants, eliminate_dead_vars, encrypted_eval, expand_arrow_sequences,
    flatten_top_level_cjs_wrapper, heuristic_rename, rename_guards, resolve_compound_transitions,
};
use annotations::annotate_webpack_modules;
use checkpoint::{checkpoint_error_context, log_output_validity};
use trace::{TraceSpan, mem_ck};

pub use trace::{ast_trace_enabled, deob_trace_enabled};

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
