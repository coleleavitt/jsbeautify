use serde::{Deserialize, Serialize};

/// Current JSON schema version for [`VmAnalysisReport`].
pub const VM_ANALYSIS_SCHEMA_VERSION: u32 = 1;

/// The structured completion state of a VM analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AnalysisOutcome {
    #[default]
    CompletedNone,
    CompletedFindings,
    Invalid {
        reason: InvalidAnalysisReason,
    },
    NotAnalyzed {
        reason: NotAnalyzedReason,
    },
    Truncated {
        reason: TruncatedAnalysisReason,
    },
}

/// A typed reason that makes the analyzed input invalid as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidAnalysisReason {
    JavaScriptSyntax,
}

/// A typed reason why analysis did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotAnalyzedReason {
    SourceTooLarge { actual_bytes: usize, limit_bytes: usize },
}

/// A typed reason why analysis returned only bounded partial results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncatedAnalysisReason {
    AggregateBudgetExhausted,
    ReportPayloadLimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsPatternKind {
    LargeLiteralPool,
    DescriptorObjectArray,
    HandlerFunctionArray,
    VmFrameStackLocals,
    WebAssemblyBridge,
    BlobWorkerPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsPatternFinding {
    pub kind: JsPatternKind,
    pub span: SourceSpan,
    pub item_count: Option<usize>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerRole {
    StackRead,
    StackWrite,
    LocalRead,
    LocalWrite,
    UnaryOperation,
    BinaryOperation,
    PropertyAccess,
    Call,
    Construct,
    StringDecode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerCandidate {
    pub name: Option<String>,
    pub span: SourceSpan,
    pub roles: Vec<HandlerRole>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAnnotationSuggestion {
    pub span: SourceSpan,
    pub suggested_name: String,
    pub roles: Vec<HandlerRole>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolRole {
    HandlerArray,
    OperandStack,
    ExceptionStack,
    LocalStore,
    DescriptorArray,
    Base64Decoder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRoleSuggestion {
    pub symbol: String,
    pub span: Option<SourceSpan>,
    pub role: SymbolRole,
    pub suggested_name: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedWasm {
    pub source_span: SourceSpan,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteralFoldKind {
    Base64Utf8,
    Base64Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteralFold {
    pub decoder: String,
    pub call_span: SourceSpan,
    pub kind: LiteralFoldKind,
    pub byte_len: usize,
    pub digest_md5: String,
    pub utf8_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmSection {
    pub kind: String,
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmImport {
    pub module: String,
    pub name: String,
    pub kind: String,
    pub type_index: Option<u32>,
    pub function_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmExport {
    pub name: String,
    pub kind: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmOperator {
    pub offset: usize,
    pub kind: String,
    pub operand: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredControlNodeKind {
    FunctionEntry,
    Block,
    Loop,
    If,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredControlNode {
    pub id: u32,
    pub kind: StructuredControlNodeKind,
    pub operator_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredControlEdgeKind {
    Contains,
    Branch,
    ConditionalBranch,
    BranchTable,
    CallReference,
    CallIndirectReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredControlEdge {
    pub from: u32,
    pub to: Option<u32>,
    pub kind: StructuredControlEdgeKind,
    pub target_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmFunctionStructure {
    pub function_index: u32,
    pub nodes: Vec<StructuredControlNode>,
    pub edges: Vec<StructuredControlEdge>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmModuleReport {
    pub source_span: SourceSpan,
    pub byte_len: usize,
    pub sections: Vec<WasmSection>,
    pub imports: Vec<WasmImport>,
    pub exports: Vec<WasmExport>,
    pub operators: Vec<WasmOperator>,
    pub structured_control: Vec<WasmFunctionStructure>,
    pub valid: bool,
    pub truncated: bool,
    pub validation_errors: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmAnalysisReport {
    pub schema_version: u32,
    pub outcome: AnalysisOutcome,
    pub patterns: Vec<JsPatternFinding>,
    pub handler_candidates: Vec<HandlerCandidate>,
    pub literal_folds: Vec<LiteralFold>,
    pub wasm_modules: Vec<WasmModuleReport>,
    pub role_suggestions: Vec<RoleAnnotationSuggestion>,
    pub symbol_role_suggestions: Vec<SymbolRoleSuggestion>,
    pub diagnostics: Vec<String>,
}

impl Default for VmAnalysisReport {
    fn default() -> Self {
        Self {
            schema_version: VM_ANALYSIS_SCHEMA_VERSION,
            outcome: AnalysisOutcome::CompletedNone,
            patterns: Vec::new(),
            handler_candidates: Vec::new(),
            literal_folds: Vec::new(),
            wasm_modules: Vec::new(),
            role_suggestions: Vec::new(),
            symbol_role_suggestions: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl VmAnalysisReport {
    pub(super) const fn has_findings(&self) -> bool {
        !self.patterns.is_empty()
            || !self.handler_candidates.is_empty()
            || !self.literal_folds.is_empty()
            || !self.wasm_modules.is_empty()
            || !self.role_suggestions.is_empty()
            || !self.symbol_role_suggestions.is_empty()
    }
}
