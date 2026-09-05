use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, Validator, WasmFeatures};

use super::budget::AnalysisBudget;
use super::literals::DecodedLiteral;
use super::report::{
    StructuredControlEdge, StructuredControlEdgeKind, StructuredControlNode, StructuredControlNodeKind, WasmExport,
    WasmFunctionStructure, WasmImport, WasmModuleReport, WasmOperator, WasmSection,
};

const MAX_SECTIONS_PER_MODULE: usize = 128;

pub(super) fn analyze_decoded_modules(
    literals: &[DecodedLiteral],
    budget: &mut AnalysisBudget,
) -> Vec<WasmModuleReport> {
    let mut reports = Vec::new();
    for literal in literals.iter().filter(|literal| literal.bytes.starts_with(b"\0asm")) {
        if !budget.retain_module() {
            break;
        }
        reports.push(analyze_module(literal, budget));
        if budget.exhausted() {
            break;
        }
    }
    reports
}

pub(super) fn is_valid_module(bytes: &[u8]) -> bool {
    Validator::new_with_features(WasmFeatures::WASM3)
        .validate_all(bytes)
        .is_ok()
}

fn analyze_module(literal: &DecodedLiteral, budget: &mut AnalysisBudget) -> WasmModuleReport {
    let validation = Validator::new_with_features(WasmFeatures::WASM3).validate_all(&literal.bytes);
    let mut report = WasmModuleReport {
        source_span: literal.span,
        byte_len: literal.bytes.len(),
        sections: Vec::new(),
        imports: Vec::new(),
        exports: Vec::new(),
        operators: Vec::new(),
        structured_control: Vec::new(),
        valid: validation.is_ok(),
        truncated: false,
        validation_errors: Vec::new(),
        errors: Vec::new(),
    };
    if let Err(error) = validation {
        if budget.retain_error() {
            report.validation_errors.push(error.to_string());
        }
        return report;
    }

    let mut imported_functions = 0_u32;
    let mut defined_function = 0_u32;
    for payload in Parser::new(0).parse_all(&literal.bytes) {
        if budget.exhausted() {
            report.truncated = true;
            break;
        }
        let payload = match payload {
            Ok(payload) => payload,
            Err(error) => {
                push_error(&mut report.errors, error.to_string(), budget);
                break;
            }
        };
        match payload {
            Payload::Version { .. } => push_section(&mut report, section("version", None)),
            Payload::TypeSection(reader) => push_section(&mut report, section("type", Some(reader.count()))),
            Payload::ImportSection(reader) => {
                push_section(&mut report, section("import", Some(reader.count())));
                for import in reader.into_imports() {
                    if !budget.retain_import_export() {
                        report.truncated = true;
                        break;
                    }
                    match import {
                        Ok(import) => {
                            let (kind, type_index, function_index) = match import.ty {
                                TypeRef::Func(index) | TypeRef::FuncExact(index) => {
                                    let function_index = imported_functions;
                                    imported_functions = imported_functions.saturating_add(1);
                                    ("function", Some(index), Some(function_index))
                                }
                                TypeRef::Table(_) => ("table", None, None),
                                TypeRef::Memory(_) => ("memory", None, None),
                                TypeRef::Global(_) => ("global", None, None),
                                TypeRef::Tag(_) => ("tag", None, None),
                            };
                            report.imports.push(WasmImport {
                                module: import.module.to_string(),
                                name: import.name.to_string(),
                                kind: kind.to_string(),
                                type_index,
                                function_index,
                            });
                        }
                        Err(error) => push_error(&mut report.errors, error.to_string(), budget),
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                push_section(&mut report, section("function", Some(reader.count())));
            }
            Payload::TableSection(reader) => push_section(&mut report, section("table", Some(reader.count()))),
            Payload::MemorySection(reader) => push_section(&mut report, section("memory", Some(reader.count()))),
            Payload::GlobalSection(reader) => push_section(&mut report, section("global", Some(reader.count()))),
            Payload::ExportSection(reader) => {
                push_section(&mut report, section("export", Some(reader.count())));
                for export in reader {
                    if !budget.retain_import_export() {
                        report.truncated = true;
                        break;
                    }
                    match export {
                        Ok(export) => report.exports.push(WasmExport {
                            name: export.name.to_string(),
                            kind: external_kind(export.kind).to_string(),
                            index: export.index,
                        }),
                        Err(error) => push_error(&mut report.errors, error.to_string(), budget),
                    }
                }
            }
            Payload::StartSection { .. } => push_section(&mut report, section("start", None)),
            Payload::ElementSection(reader) => push_section(&mut report, section("element", Some(reader.count()))),
            Payload::DataCountSection { count, .. } => {
                push_section(&mut report, section("data_count", Some(count)));
            }
            Payload::DataSection(reader) => push_section(&mut report, section("data", Some(reader.count()))),
            Payload::CodeSectionStart { count, .. } => push_section(&mut report, section("code", Some(count))),
            Payload::CodeSectionEntry(body) => {
                if !budget.retain_function() {
                    report.truncated = true;
                    break;
                }
                let function_index = imported_functions.saturating_add(defined_function);
                defined_function = defined_function.saturating_add(1);
                match parse_function(&body, function_index, &mut report.operators, budget) {
                    Ok(structure) => {
                        report.truncated |= structure.truncated;
                        report.structured_control.push(structure);
                    }
                    Err(error) => push_error(&mut report.errors, error.to_string(), budget),
                }
            }
            Payload::CustomSection(reader) => push_section(
                &mut report,
                WasmSection {
                    kind: format!("custom:{}", reader.name()),
                    count: None,
                },
            ),
            Payload::TagSection(reader) => push_section(&mut report, section("tag", Some(reader.count()))),
            Payload::End(_) => {}
            _ => push_section(&mut report, section("component_or_unknown", None)),
        }
    }
    report
}

fn parse_function(
    body: &wasmparser::FunctionBody<'_>,
    function_index: u32,
    all_operators: &mut Vec<WasmOperator>,
    budget: &mut AnalysisBudget,
) -> wasmparser::Result<WasmFunctionStructure> {
    let entry_emitted = budget.retain_graph_node();
    let mut structure = WasmFunctionStructure {
        function_index,
        nodes: entry_emitted
            .then(|| StructuredControlNode {
                id: 0,
                kind: StructuredControlNodeKind::FunctionEntry,
                operator_offset: body.range().start,
            })
            .into_iter()
            .collect(),
        edges: Vec::new(),
        truncated: !entry_emitted,
    };
    // Labels are tracked independently from emitted nodes. This remains
    // balanced even when graph/report emission has been truncated.
    let mut active_labels = vec![entry_emitted.then_some(0_u32)];
    let mut reader = body.get_operators_reader()?;
    while !reader.eof() {
        let offset = reader.original_position();
        let operator = reader.read()?;
        if !budget.exhausted() && budget.visit_operator() {
            let (kind, operand) = operator_summary(&operator);
            all_operators.push(WasmOperator { offset, kind, operand });
        } else {
            structure.truncated = true;
        }
        let current = active_labels.last().copied().flatten();
        match operator {
            Operator::Block { .. } | Operator::Loop { .. } | Operator::If { .. } => {
                let node_kind = match operator {
                    Operator::Loop { .. } => StructuredControlNodeKind::Loop,
                    Operator::If { .. } => StructuredControlNodeKind::If,
                    _ => StructuredControlNodeKind::Block,
                };
                let emitted_id = if !budget.exhausted() && budget.retain_graph_node() {
                    let id = u32::try_from(structure.nodes.len()).unwrap_or(u32::MAX);
                    structure.nodes.push(StructuredControlNode {
                        id,
                        kind: node_kind,
                        operator_offset: offset,
                    });
                    push_edge(
                        &mut structure,
                        current,
                        Some(id),
                        StructuredControlEdgeKind::Contains,
                        None,
                        budget,
                    );
                    Some(id)
                } else {
                    structure.truncated = true;
                    None
                };
                active_labels.push(emitted_id);
            }
            Operator::End => {
                if active_labels.len() > 1 {
                    active_labels.pop();
                }
            }
            Operator::Br { relative_depth } => push_branch(
                &mut structure,
                &active_labels,
                current,
                relative_depth,
                StructuredControlEdgeKind::Branch,
                budget,
            ),
            Operator::BrIf { relative_depth } => push_branch(
                &mut structure,
                &active_labels,
                current,
                relative_depth,
                StructuredControlEdgeKind::ConditionalBranch,
                budget,
            ),
            Operator::BrTable { targets } => {
                for depth in targets.targets().chain(std::iter::once(Ok(targets.default()))) {
                    push_branch(
                        &mut structure,
                        &active_labels,
                        current,
                        depth?,
                        StructuredControlEdgeKind::BranchTable,
                        budget,
                    );
                }
            }
            Operator::Call { function_index } | Operator::ReturnCall { function_index } => push_edge(
                &mut structure,
                current,
                None,
                StructuredControlEdgeKind::CallReference,
                Some(function_index),
                budget,
            ),
            Operator::CallIndirect { type_index, .. } | Operator::ReturnCallIndirect { type_index, .. } => push_edge(
                &mut structure,
                current,
                None,
                StructuredControlEdgeKind::CallIndirectReference,
                Some(type_index),
                budget,
            ),
            _ => {}
        }
    }
    Ok(structure)
}

fn push_branch(
    structure: &mut WasmFunctionStructure,
    labels: &[Option<u32>],
    from: Option<u32>,
    depth: u32,
    kind: StructuredControlEdgeKind,
    budget: &mut AnalysisBudget,
) {
    let depth = usize::try_from(depth).unwrap_or(usize::MAX);
    let target = labels
        .len()
        .checked_sub(depth.saturating_add(1))
        .and_then(|index| labels.get(index))
        .copied()
        .flatten();
    push_edge(structure, from, target, kind, None, budget);
}

fn push_edge(
    structure: &mut WasmFunctionStructure,
    from: Option<u32>,
    to: Option<u32>,
    kind: StructuredControlEdgeKind,
    target_index: Option<u32>,
    budget: &mut AnalysisBudget,
) {
    let Some(from) = from else {
        structure.truncated = true;
        return;
    };
    if !budget.exhausted() && budget.retain_graph_edge() {
        structure.edges.push(StructuredControlEdge {
            from,
            to,
            kind,
            target_index,
        });
    } else {
        structure.truncated = true;
    }
}

fn push_section(report: &mut WasmModuleReport, section: WasmSection) {
    if report.sections.len() < MAX_SECTIONS_PER_MODULE {
        report.sections.push(section);
    } else {
        report.truncated = true;
    }
}

fn push_error(errors: &mut Vec<String>, error: String, budget: &mut AnalysisBudget) {
    if budget.retain_error() {
        errors.push(error);
    }
}

fn operator_summary(operator: &Operator<'_>) -> (String, Option<u32>) {
    let operand = match operator {
        Operator::Br { relative_depth } | Operator::BrIf { relative_depth } => Some(*relative_depth),
        Operator::Call { function_index } | Operator::ReturnCall { function_index } => Some(*function_index),
        Operator::CallIndirect { type_index, .. } | Operator::ReturnCallIndirect { type_index, .. } => {
            Some(*type_index)
        }
        Operator::LocalGet { local_index }
        | Operator::LocalSet { local_index }
        | Operator::LocalTee { local_index } => Some(*local_index),
        Operator::GlobalGet { global_index } | Operator::GlobalSet { global_index } => Some(*global_index),
        _ => None,
    };
    let debug = format!("{operator:?}");
    let kind = debug.split([' ', '{']).next().unwrap_or("unknown").to_string();
    (kind, operand)
}

fn section(kind: &str, count: Option<u32>) -> WasmSection {
    WasmSection {
        kind: kind.to_string(),
        count,
    }
}

const fn external_kind(kind: ExternalKind) -> &'static str {
    match kind {
        ExternalKind::Func | ExternalKind::FuncExact => "function",
        ExternalKind::Table => "table",
        ExternalKind::Memory => "memory",
        ExternalKind::Global => "global",
        ExternalKind::Tag => "tag",
    }
}
