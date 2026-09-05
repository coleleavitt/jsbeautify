use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use super::report::{
    HandlerCandidate, HandlerRole, JsPatternFinding, JsPatternKind, RoleAnnotationSuggestion, SourceSpan, SymbolRole,
    SymbolRoleSuggestion,
};
use super::syntax::{ArrayFact, SyntaxFacts};

const LARGE_LITERAL_POOL_MIN: usize = 64;
const STRUCTURED_ARRAY_MIN: usize = 4;
const MEMBER_EVIDENCE_MIN: usize = 8;
const MAX_HANDLER_CANDIDATES: usize = 512;

#[derive(Default)]
pub(super) struct JavaScriptAnalysis {
    pub(super) patterns: Vec<JsPatternFinding>,
    pub(super) handlers: Vec<HandlerCandidate>,
    pub(super) handler_suggestions: Vec<RoleAnnotationSuggestion>,
    pub(super) symbol_suggestions: Vec<SymbolRoleSuggestion>,
    pub(super) diagnostics: Vec<String>,
}

pub(super) fn analyze(source: &str, facts: &SyntaxFacts) -> JavaScriptAnalysis {
    let mut patterns = Vec::new();
    let mut symbol_suggestions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut handler_array = None;

    for array in &facts.arrays {
        if array.item_count >= LARGE_LITERAL_POOL_MIN && array.string_count * 5 >= array.item_count * 4 {
            patterns.push(array_finding(
                array,
                JsPatternKind::LargeLiteralPool,
                vec![format!("{} direct string literal elements", array.string_count)],
            ));
        }
        if array.item_count >= STRUCTURED_ARRAY_MIN && array.object_count * 5 >= array.item_count * 4 {
            patterns.push(array_finding(
                array,
                JsPatternKind::DescriptorObjectArray,
                vec![format!("{} direct object literal elements", array.object_count)],
            ));
            if let Some(symbol) = &array.symbol {
                symbol_suggestions.push(symbol_suggestion(
                    symbol,
                    array.symbol_span,
                    SymbolRole::DescriptorArray,
                    "vm_descriptors",
                    format!("array has {} descriptor-like object elements", array.object_count),
                ));
            }
        }
        if array.item_count >= STRUCTURED_ARRAY_MIN && array.function_spans.len() * 5 >= array.item_count * 4 {
            patterns.push(array_finding(
                array,
                JsPatternKind::HandlerFunctionArray,
                vec![format!("{} direct function elements", array.function_spans.len())],
            ));
            if handler_array.is_none()
                || array.function_spans.len()
                    > handler_array.map_or(0, |candidate: &ArrayFact| candidate.function_spans.len())
            {
                handler_array = Some(array);
            }
        }
    }

    add_global_patterns(source, &mut patterns);

    let (handlers, handler_suggestions) = if let Some(array) = handler_array {
        if let Some(symbol) = &array.symbol {
            symbol_suggestions.push(symbol_suggestion(
                symbol,
                array.symbol_span,
                SymbolRole::HandlerArray,
                "vm_handlers",
                format!("array has {} direct function elements", array.function_spans.len()),
            ));
        }
        let inferred = infer_frame_members(source, array);
        diagnostics.extend(inferred.diagnostics.clone());
        if inferred.operand_stack.is_some() && inferred.locals.is_some() {
            patterns.push(JsPatternFinding {
                kind: JsPatternKind::VmFrameStackLocals,
                span: array.span,
                item_count: None,
                evidence: vec![
                    "repeated inferred operand-stack access".to_string(),
                    "repeated inferred local-store reads/writes".to_string(),
                ],
            });
        }
        add_member_suggestions(source, array, &inferred, &mut symbol_suggestions);
        classify_handler_array(source, array, &inferred)
    } else {
        diagnostics.push("no function-dense handler array met the structural threshold".to_string());
        (Vec::new(), Vec::new())
    };

    JavaScriptAnalysis {
        patterns,
        handlers,
        handler_suggestions,
        symbol_suggestions,
        diagnostics,
    }
}

const fn array_finding(array: &ArrayFact, kind: JsPatternKind, evidence: Vec<String>) -> JsPatternFinding {
    JsPatternFinding {
        kind,
        span: array.span,
        item_count: Some(array.item_count),
        evidence,
    }
}

fn add_global_patterns(source: &str, findings: &mut Vec<JsPatternFinding>) {
    let whole = SourceSpan {
        start: 0,
        end: source.len(),
    };
    let frame_terms = present_terms(source, &["stack", "locals", "frame"]);
    if frame_terms.len() >= 2 && (source.contains(".push(") || source.contains(".pop(") || source.contains("stack[")) {
        findings.push(JsPatternFinding {
            kind: JsPatternKind::VmFrameStackLocals,
            span: whole,
            item_count: None,
            evidence: frame_terms,
        });
    }
    let wasm_terms = present_terms(
        source,
        &["WebAssembly.Module", "WebAssembly.Instance", "WebAssembly.instantiate"],
    );
    if !wasm_terms.is_empty() {
        findings.push(JsPatternFinding {
            kind: JsPatternKind::WebAssemblyBridge,
            span: whole,
            item_count: None,
            evidence: wasm_terms,
        });
    }
    let worker_terms = present_terms(source, &["Blob", "Worker", "URL.createObjectURL"]);
    if worker_terms.iter().any(|term| term == "Blob") && worker_terms.iter().any(|term| term == "Worker") {
        findings.push(JsPatternFinding {
            kind: JsPatternKind::BlobWorkerPath,
            span: whole,
            item_count: None,
            evidence: worker_terms,
        });
    }
}

#[derive(Default)]
struct InferredMembers {
    operand_stack: Option<String>,
    exception_stack: Option<String>,
    locals: Option<String>,
    local_read_methods: BTreeSet<String>,
    local_write_methods: BTreeSet<String>,
    string_decoders: BTreeSet<String>,
    call_helpers: BTreeSet<String>,
    diagnostics: Vec<String>,
}

fn infer_frame_members(source: &str, array: &ArrayFact) -> InferredMembers {
    let mut inferred = InferredMembers::default();
    let handler_source = source.get(array.span.start..array.span.end).unwrap_or_default();
    let indexed_member =
        Regex::new(r"\b[A-Za-z_$][\w$]*\.([A-Za-z_$][\w$]*)\s*(?:\[|\.length\b)").expect("valid indexed member regex");
    let mut indexed_counts = BTreeMap::<String, usize>::new();
    for captures in indexed_member.captures_iter(handler_source) {
        if let Some(member) = captures.get(1) {
            *indexed_counts.entry(member.as_str().to_string()).or_default() += 1;
        }
    }
    inferred.operand_stack = indexed_counts
        .into_iter()
        .filter(|(_, count)| *count >= MEMBER_EVIDENCE_MIN)
        .max_by_key(|(_, count)| *count)
        .map(|(member, _)| member);

    let member_call = Regex::new(r"\b[A-Za-z_$][\w$]*\.([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)\s*\(([^()]*)\)")
        .expect("valid nested member call regex");
    let mut call_counts = BTreeMap::<String, BTreeMap<String, (usize, BTreeSet<usize>)>>::new();
    for captures in member_call.captures_iter(handler_source) {
        let (Some(member), Some(method), Some(arguments)) = (captures.get(1), captures.get(2), captures.get(3)) else {
            continue;
        };
        let arity = argument_count(arguments.as_str());
        let entry = call_counts
            .entry(member.as_str().to_string())
            .or_default()
            .entry(method.as_str().to_string())
            .or_default();
        entry.0 += 1;
        entry.1.insert(arity);
    }
    if let Some((member, methods)) = call_counts
        .into_iter()
        .filter(|(_, methods)| methods.values().map(|(count, _)| count).sum::<usize>() >= MEMBER_EVIDENCE_MIN)
        .max_by_key(|(_, methods)| methods.values().map(|(count, _)| count).sum::<usize>())
    {
        for (method, (_, arities)) in methods {
            if arities.iter().any(|arity| *arity >= 2) {
                inferred.local_write_methods.insert(method);
            } else if arities.contains(&1) {
                inferred.local_read_methods.insert(method);
            }
        }
        if !inferred.local_read_methods.is_empty() && !inferred.local_write_methods.is_empty() {
            inferred.locals = Some(member);
        }
    }

    inferred.string_decoders = string_decoder_aliases(source);
    inferred.call_helpers = call_helper_aliases(source);
    if let Some(operand_stack) = &inferred.operand_stack {
        inferred.exception_stack = infer_exception_stack(source, operand_stack);
    }
    if let Some(member) = &inferred.operand_stack {
        inferred.diagnostics.push(format!(
            "inferred operand-stack member `{member}` from repeated indexed/length usage"
        ));
    }
    if let Some(member) = &inferred.locals {
        inferred.diagnostics.push(format!(
            "inferred local-store member `{member}` with read methods {:?} and write methods {:?}",
            inferred.local_read_methods, inferred.local_write_methods
        ));
    }
    if let Some(member) = &inferred.exception_stack {
        inferred.diagnostics.push(format!(
            "inferred exception-stack member `{member}` from sibling stack construction and catch usage"
        ));
    }
    inferred
}

fn infer_exception_stack(source: &str, operand_stack: &str) -> Option<String> {
    let factory_pattern = format!(
        r"this\.{}\s*=\s*([A-Za-z_$][\w$]*)\s*\(\s*\)",
        regex::escape(operand_stack)
    );
    let factory_re = Regex::new(&factory_pattern).ok()?;
    let factory = factory_re.captures(source)?.get(1)?.as_str();
    let sibling_pattern = format!(r"this\.([A-Za-z_$][\w$]*)\s*=\s*{}\s*\(\s*\)", regex::escape(factory));
    let sibling_re = Regex::new(&sibling_pattern).ok()?;
    sibling_re
        .captures_iter(source)
        .filter_map(|captures| captures.get(1))
        .map(|member| member.as_str())
        .find(|member| {
            *member != operand_stack
                && (source.contains(&format!(".{member}.cN(")) || source.contains(&format!(".{member}[")))
        })
        .map(str::to_string)
}

fn string_decoder_aliases(source: &str) -> BTreeSet<String> {
    let alias_re = Regex::new(r"\b([A-Za-z_$][\w$]*)\s*=\s*[A-Za-z_$][\w$]*\.fromCharCode\b")
        .expect("valid string decoder alias regex");
    alias_re
        .captures_iter(source)
        .filter_map(|captures| captures.get(1))
        .map(|alias| alias.as_str().to_string())
        .collect()
}

fn call_helper_aliases(source: &str) -> BTreeSet<String> {
    let direct_re =
        Regex::new(r"\b([A-Za-z_$][\w$]*)\s*=\s*[A-Za-z_$][\w$]*\.(?:apply|call)\b").expect("valid call helper regex");
    let direct = direct_re
        .captures_iter(source)
        .filter_map(|captures| captures.get(1))
        .map(|alias| alias.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut helpers = direct.clone();
    let assignment_re = Regex::new(r"\b([A-Za-z_$][\w$]*)\s*=\s*([A-Za-z_$][\w$]*)\s*\(([^()]*)\)")
        .expect("valid helper assignment regex");
    for captures in assignment_re.captures_iter(source) {
        let (Some(alias), Some(arguments)) = (captures.get(1), captures.get(3)) else {
            continue;
        };
        if direct
            .iter()
            .any(|helper| arguments.as_str().split(',').any(|argument| argument.trim() == helper))
        {
            helpers.insert(alias.as_str().to_string());
        }
    }
    helpers
}

fn classify_handler_array(
    source: &str,
    array: &ArrayFact,
    inferred: &InferredMembers,
) -> (Vec<HandlerCandidate>, Vec<RoleAnnotationSuggestion>) {
    let mut handlers = Vec::new();
    for (index, span) in array.function_spans.iter().take(MAX_HANDLER_CANDIDATES).enumerate() {
        let body = source.get(span.start..span.end).unwrap_or_default();
        let (roles, evidence) = classify_body(body, inferred);
        handlers.push(HandlerCandidate {
            name: array.symbol.as_ref().map(|symbol| format!("{symbol}[{index}]")),
            span: *span,
            roles,
            evidence,
        });
    }
    let suggestions = handlers
        .iter()
        .filter(|handler| !handler.roles.is_empty())
        .map(|handler| RoleAnnotationSuggestion {
            span: handler.span,
            suggested_name: suggested_name(&handler.roles).to_string(),
            roles: handler.roles.clone(),
            rationale: format!(
                "Static handler syntax contains {}; suggestion only; source is not rewritten",
                handler.evidence.join(", ")
            ),
        })
        .collect();
    (handlers, suggestions)
}

fn classify_body(body: &str, inferred: &InferredMembers) -> (Vec<HandlerRole>, Vec<String>) {
    let mut roles = BTreeSet::new();
    let mut evidence = Vec::new();
    if let Some(stack) = &inferred.operand_stack {
        let access = format!(".{stack}[");
        let length = format!(".{stack}.length");
        if body.contains(&access) || body.contains(&length) {
            add_role(&mut roles, &mut evidence, HandlerRole::StackRead, "operand-stack read");
        }
        if assignment_to_member(body, stack) || length_is_written(body, stack) {
            add_role(
                &mut roles,
                &mut evidence,
                HandlerRole::StackWrite,
                "operand-stack write",
            );
        }
    }
    if let Some(locals) = &inferred.locals {
        if inferred
            .local_read_methods
            .iter()
            .any(|method| body.contains(&format!(".{locals}.{method}(")))
        {
            add_role(&mut roles, &mut evidence, HandlerRole::LocalRead, "local-store read");
        }
        if inferred
            .local_write_methods
            .iter()
            .any(|method| body.contains(&format!(".{locals}.{method}(")))
        {
            add_role(&mut roles, &mut evidence, HandlerRole::LocalWrite, "local-store write");
        }
    }
    if ["typeof ", "void ", "delete ", "!", "~"]
        .iter()
        .any(|token| body.contains(token))
    {
        add_role(&mut roles, &mut evidence, HandlerRole::UnaryOperation, "unary operator");
    }
    if [
        " + ", " - ", " * ", " / ", " === ", " == ", " < ", " > ", " & ", " | ", " ^ ",
    ]
    .iter()
    .any(|token| body.contains(token))
    {
        add_role(
            &mut roles,
            &mut evidence,
            HandlerRole::BinaryOperation,
            "binary operator",
        );
    }
    let dynamic_property = Regex::new(r"\[[A-Za-z_$][\w$]*\]").expect("valid dynamic property regex");
    if dynamic_property.is_match(body) {
        add_role(
            &mut roles,
            &mut evidence,
            HandlerRole::PropertyAccess,
            "computed property access",
        );
    }
    if body.contains(".apply(")
        || body.contains(".call(")
        || body.contains("Reflect.apply(")
        || inferred
            .call_helpers
            .iter()
            .any(|helper| body.contains(&format!("{helper}(")))
    {
        add_role(&mut roles, &mut evidence, HandlerRole::Call, "call/apply helper");
    }
    if body.contains("new ") || body.contains("Reflect.construct") {
        add_role(&mut roles, &mut evidence, HandlerRole::Construct, "construction");
    }
    if ["TextDecoder", "fromCharCode", "decodeURIComponent", "atob("]
        .iter()
        .any(|token| body.contains(token))
        || inferred
            .string_decoders
            .iter()
            .any(|alias| body.contains(&format!("{alias}(")))
    {
        add_role(&mut roles, &mut evidence, HandlerRole::StringDecode, "string decoding");
    }
    (roles.into_iter().collect(), evidence)
}

fn add_member_suggestions(
    source: &str,
    array: &ArrayFact,
    inferred: &InferredMembers,
    suggestions: &mut Vec<SymbolRoleSuggestion>,
) {
    if let Some(symbol) = &inferred.operand_stack {
        suggestions.push(member_suggestion(
            source,
            array.span,
            symbol,
            SymbolRole::OperandStack,
            "operand_stack",
            "repeated indexed and length-based access across handlers",
        ));
    }
    if let Some(symbol) = &inferred.exception_stack {
        suggestions.push(member_suggestion(
            source,
            SourceSpan {
                start: 0,
                end: source.len(),
            },
            symbol,
            SymbolRole::ExceptionStack,
            "exception_stack",
            "constructed by the operand-stack factory and used by catch/throw bridges",
        ));
    }
    if let Some(symbol) = &inferred.locals {
        suggestions.push(member_suggestion(
            source,
            array.span,
            symbol,
            SymbolRole::LocalStore,
            "locals",
            "repeated one-argument reads and multi-argument writes across handlers",
        ));
    }
}

fn symbol_suggestion(
    symbol: &str,
    span: Option<SourceSpan>,
    role: SymbolRole,
    suggested_name: &str,
    rationale: String,
) -> SymbolRoleSuggestion {
    SymbolRoleSuggestion {
        symbol: symbol.to_string(),
        span,
        role,
        suggested_name: suggested_name.to_string(),
        rationale,
    }
}

fn member_suggestion(
    _source: &str,
    _within: SourceSpan,
    symbol: &str,
    role: SymbolRole,
    suggested_name: &str,
    rationale: &str,
) -> SymbolRoleSuggestion {
    SymbolRoleSuggestion {
        symbol: symbol.to_string(),
        span: None,
        role,
        suggested_name: suggested_name.to_string(),
        rationale: rationale.to_string(),
    }
}

fn assignment_to_member(body: &str, member: &str) -> bool {
    let pattern = format!(r"\.{}\s*\[[^\]]+\]\s*=", regex::escape(member));
    Regex::new(&pattern).is_ok_and(|regex| regex.is_match(body))
}

fn length_is_written(body: &str, member: &str) -> bool {
    let pattern = format!(r"\.{}\.length\s*(?:[+\-*/]?=|\+\+|--)", regex::escape(member));
    Regex::new(&pattern).is_ok_and(|regex| regex.is_match(body))
}

fn argument_count(arguments: &str) -> usize {
    if arguments.trim().is_empty() {
        0
    } else {
        arguments.bytes().filter(|byte| *byte == b',').count() + 1
    }
}

fn add_role(roles: &mut BTreeSet<HandlerRole>, evidence: &mut Vec<String>, role: HandlerRole, reason: &str) {
    if roles.insert(role) {
        evidence.push(reason.to_string());
    }
}

fn suggested_name(roles: &[HandlerRole]) -> &'static str {
    if roles.contains(&HandlerRole::StringDecode) {
        "vm_string_decoder"
    } else if roles.contains(&HandlerRole::Construct) {
        "vm_construct_handler"
    } else if roles.contains(&HandlerRole::Call) {
        "vm_call_handler"
    } else if roles.contains(&HandlerRole::PropertyAccess) {
        "vm_property_handler"
    } else if roles.contains(&HandlerRole::LocalWrite) {
        "vm_local_write_handler"
    } else if roles.contains(&HandlerRole::LocalRead) {
        "vm_local_read_handler"
    } else if roles.contains(&HandlerRole::StackWrite) {
        "vm_stack_write_handler"
    } else if roles.contains(&HandlerRole::StackRead) {
        "vm_stack_read_handler"
    } else if roles.contains(&HandlerRole::BinaryOperation) {
        "vm_binary_handler"
    } else {
        "vm_unary_handler"
    }
}

fn present_terms(source: &str, terms: &[&str]) -> Vec<String> {
    terms
        .iter()
        .filter(|term| source.contains(**term))
        .map(|term| (*term).to_string())
        .collect()
}
