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
    ExtractedWasm, HandlerCandidate, HandlerRole, JsPatternFinding, JsPatternKind, LiteralFold, LiteralFoldKind,
    RoleAnnotationSuggestion, SourceSpan, StructuredControlEdge, StructuredControlEdgeKind, StructuredControlNode,
    StructuredControlNodeKind, SymbolRole, SymbolRoleSuggestion, VmAnalysisReport, WasmExport, WasmFunctionStructure,
    WasmImport, WasmModuleReport, WasmOperator, WasmSection,
};

/// Analyze JavaScript source without executing it or applying transformations.
#[must_use]
pub fn analyze(source: &str) -> VmAnalysisReport {
    analyze_with_limits(source, AnalysisLimits::default())
}

/// Analyze with explicit aggregate hostile-input limits.
#[must_use]
pub fn analyze_with_limits(source: &str, limits: AnalysisLimits) -> VmAnalysisReport {
    if source.len() > limits.max_source_bytes {
        return VmAnalysisReport {
            diagnostics: vec![format!(
                "analysis truncated: source is {} bytes; limit is {} bytes",
                source.len(),
                limits.max_source_bytes
            )],
            ..VmAnalysisReport::default()
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
    let wasm_modules = if budget.exhausted() {
        Vec::new()
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
        wasm_modules,
        role_suggestions: javascript.handler_suggestions,
        symbol_role_suggestions,
        diagnostics,
    };
    enforce_report_payload(&mut report, &mut budget);
    if budget.exhausted() {
        report
            .diagnostics
            .retain(|diagnostic| !diagnostic.starts_with("analysis truncated:"));
        report
            .diagnostics
            .push("analysis truncated: aggregate hostile-input budget exhausted".to_string());
    }
    report
}

/// Extract only validated literal WebAssembly binaries passed through
/// recognized local base64 decoders. No JavaScript or WebAssembly code is
/// executed.
#[must_use]
pub fn extract_wasm(source: &str) -> Vec<ExtractedWasm> {
    let limits = AnalysisLimits::default();
    if source.len() > limits.max_source_bytes {
        return Vec::new();
    }
    let mut budget = budget::AnalysisBudget::new(limits);
    let syntax = syntax::collect(source, &mut budget);
    if budget.exhausted() {
        return Vec::new();
    }
    let literals = literals::fold_local_base64_calls(&syntax, &mut budget);
    let mut extracted = Vec::new();
    for literal in literals.decoded {
        if budget.exhausted() || !budget.retain_module() {
            break;
        }
        if wasm::is_valid_module(&literal.bytes) {
            extracted.push(ExtractedWasm {
                source_span: literal.span,
                bytes: literal.bytes,
            });
        }
    }
    extracted
}

fn enforce_report_payload(report: &mut VmAnalysisReport, budget: &mut budget::AnalysisBudget) {
    let limit = budget.limits.max_report_bytes;
    if serde_json::to_vec(report).is_ok_and(|bytes| bytes.len() <= limit) {
        return;
    }
    budget.exhaust();
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
