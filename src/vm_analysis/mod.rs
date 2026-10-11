//! Read-only structural analysis for JavaScript-hosted virtual machines.
//!
//! This module never executes JavaScript, instantiates WebAssembly, or changes
//! source text. Its output consists only of bounded static findings and naming
//! suggestions.

mod budget;
mod javascript;
mod literals;
mod report;
mod syntax;
mod wasm;

pub use budget::AnalysisLimits;
pub use report::{
    AnalysisOutcome, ExtractedWasm, HandlerCandidate, HandlerRole, InvalidAnalysisReason, JsPatternFinding,
    JsPatternKind, LiteralFold, LiteralFoldKind, NotAnalyzedReason, RoleAnnotationSuggestion, SourceSpan,
    StructuredControlEdge, StructuredControlEdgeKind, StructuredControlNode, StructuredControlNodeKind, SymbolRole,
    SymbolRoleSuggestion, TruncatedAnalysisReason, VM_ANALYSIS_SCHEMA_VERSION, VmAnalysisReport, WasmExport,
    WasmFunctionStructure, WasmImport, WasmModuleReport, WasmOperator, WasmSection,
};

/// One bounded analysis result and its exact validated WebAssembly artifacts.
///
/// `extracted_wasm` is intentionally excluded from report serialization. CLI
/// consumers write these in-memory bytes directly instead of parsing and
/// decoding the input again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmAnalysisArtifact {
    pub report: VmAnalysisReport,
    pub extracted_wasm: Vec<ExtractedWasm>,
}

/// Analyze JavaScript source without executing it or applying transformations.
#[must_use]
pub fn analyze(source: &str) -> VmAnalysisReport {
    analyze_artifact(source).report
}

/// Analyze with explicit aggregate hostile-input limits.
#[must_use]
pub fn analyze_with_limits(source: &str, limits: AnalysisLimits) -> VmAnalysisReport {
    analyze_artifact_with_limits(source, limits).report
}

/// Produce one report plus the exact validated WebAssembly found during it.
#[must_use]
pub fn analyze_artifact(source: &str) -> VmAnalysisArtifact {
    analyze_artifact_with_limits(source, AnalysisLimits::default())
}

/// Produce one artifact under a single aggregate hostile-input budget.
#[must_use]
pub fn analyze_artifact_with_limits(source: &str, limits: AnalysisLimits) -> VmAnalysisArtifact {
    crate::stack::run_on_sized_stack(source.len(), || {
        analyze_artifact_with_limits_on_current_stack(source, limits)
    })
}

fn analyze_artifact_with_limits_on_current_stack(source: &str, limits: AnalysisLimits) -> VmAnalysisArtifact {
    if source.len() > limits.max_source_bytes {
        return VmAnalysisArtifact {
            report: VmAnalysisReport {
                outcome: AnalysisOutcome::NotAnalyzed {
                    reason: NotAnalyzedReason::SourceTooLarge {
                        actual_bytes: source.len(),
                        limit_bytes: limits.max_source_bytes,
                    },
                },
                ..VmAnalysisReport::default()
            },
            extracted_wasm: Vec::new(),
        };
    }

    let mut budget = budget::AnalysisBudget::new(limits);
    let syntax = syntax::collect(source, &mut budget);
    let javascript = if budget.exhausted() {
        javascript::JavaScriptAnalysis::default()
    } else {
        javascript::analyze(source, &syntax)
    };
    let literals = if budget.exhausted() {
        literals::LiteralAnalysis::default()
    } else {
        literals::fold_local_base64_calls(&syntax, &mut budget)
    };
    let wasm = if budget.exhausted() {
        wasm::WasmAnalysis {
            reports: Vec::new(),
            extracted: Vec::new(),
        }
    } else {
        wasm::analyze_decoded_modules(&literals.decoded, &mut budget)
    };
    let mut diagnostics = syntax.diagnostics;
    diagnostics.extend(javascript.diagnostics);
    diagnostics.extend(literals.diagnostics);
    let mut symbol_role_suggestions = javascript.symbol_suggestions;
    symbol_role_suggestions.extend(literals.decoder_suggestions);

    let mut report = VmAnalysisReport {
        patterns: javascript.patterns,
        handler_candidates: javascript.handlers,
        literal_folds: literals.folds,
        wasm_modules: wasm.reports,
        role_suggestions: javascript.handler_suggestions,
        symbol_role_suggestions,
        diagnostics,
        ..VmAnalysisReport::default()
    };
    report.outcome = if !syntax.valid_program {
        AnalysisOutcome::Invalid {
            reason: InvalidAnalysisReason::JavaScriptSyntax,
        }
    } else if budget.exhausted() {
        AnalysisOutcome::Truncated {
            reason: TruncatedAnalysisReason::AggregateBudgetExhausted,
        }
    } else if report.has_findings() {
        AnalysisOutcome::CompletedFindings
    } else {
        AnalysisOutcome::CompletedNone
    };
    enforce_report_payload(&mut report, &mut budget);
    VmAnalysisArtifact {
        report,
        extracted_wasm: wasm.extracted,
    }
}

/// Extract validated literal WebAssembly binaries through a compatibility API.
///
/// This performs the same single bounded analysis as [`analyze_artifact`] and
/// returns the exact validated bytes retained by that result.
#[must_use]
pub fn extract_wasm(source: &str) -> Vec<ExtractedWasm> {
    analyze_artifact(source).extracted_wasm
}

fn enforce_report_payload(report: &mut VmAnalysisReport, budget: &mut budget::AnalysisBudget) {
    let limit = budget.limits.max_report_bytes;
    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() <= limit) {
        return;
    }
    budget.exhaust();
    report.outcome = AnalysisOutcome::Truncated {
        reason: TruncatedAnalysisReason::ReportPayloadLimitExceeded,
    };
    for module in &mut report.wasm_modules {
        module.operators.clear();
        module.structured_control.clear();
        module.truncated = true;
    }
    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() <= limit) {
        return;
    }
    report.handler_candidates.clear();
    report.role_suggestions.clear();
    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() <= limit) {
        return;
    }
    for module in &mut report.wasm_modules {
        module.imports.clear();
        module.exports.clear();
    }
    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() <= limit) {
        return;
    }
    report.patterns.clear();
    report.literal_folds.clear();
    report.symbol_role_suggestions.clear();
    report.diagnostics.clear();
}

#[cfg(test)]
mod tests;
