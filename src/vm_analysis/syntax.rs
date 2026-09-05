use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpression, AssignmentExpression, AssignmentTarget, BinaryExpression, BindingPattern,
    CallExpression, Expression, Function, FunctionBody, ReturnStatement, Statement, StringLiteral, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::operator::BinaryOperator;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use regex::Regex;

use super::budget::AnalysisBudget;
use super::report::SourceSpan;

#[derive(Debug, Clone)]
pub(super) struct ArrayFact {
    pub(super) span: SourceSpan,
    pub(super) symbol: Option<String>,
    pub(super) symbol_span: Option<SourceSpan>,
    pub(super) item_count: usize,
    pub(super) string_count: usize,
    pub(super) object_count: usize,
    pub(super) function_spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone)]
pub(super) struct DecoderFact {
    pub(super) name: String,
    pub(super) name_span: SourceSpan,
    pub(super) symbol_id: SymbolId,
    pub(super) rationale: String,
}

#[derive(Debug, Clone)]
pub(super) struct DecoderCallFact {
    pub(super) decoder: String,
    pub(super) span: SourceSpan,
    pub(super) literal: String,
}

#[derive(Debug, Default)]
pub(super) struct SyntaxFacts {
    pub(super) arrays: Vec<ArrayFact>,
    pub(super) decoder_calls: Vec<DecoderCallFact>,
    pub(super) decoders: Vec<DecoderFact>,
    pub(super) diagnostics: Vec<String>,
}

pub(super) fn collect(source: &str, budget: &mut AnalysisBudget) -> SyntaxFacts {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs()).parse();
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let scoping = semantic.scoping();

    let mut collector = SyntaxCollector {
        source,
        budget,
        arrays: Vec::new(),
        array_symbols: BTreeMap::new(),
    };
    collector.visit_program(&parsed.program);
    let mut facts = SyntaxFacts {
        arrays: collector.arrays,
        diagnostics: Vec::new(),
        ..SyntaxFacts::default()
    };
    if !parsed.diagnostics.is_empty() {
        facts.diagnostics.push(format!(
            "JavaScript parser recovered from {} diagnostic(s)",
            parsed.diagnostics.len()
        ));
    }
    if budget.exhausted() {
        return facts;
    }

    let mut definitions = DecoderDefinitionCollector {
        source,
        scoping,
        budget,
        decoders: Vec::new(),
    };
    definitions.visit_program(&parsed.program);
    facts.decoders = definitions.decoders;
    if budget.exhausted() {
        return facts;
    }

    let decoder_symbols = facts
        .decoders
        .iter()
        .map(|decoder| (decoder.symbol_id, decoder.name.clone()))
        .collect();
    let mut calls = DecoderCallCollector {
        scoping,
        budget,
        decoder_symbols,
        calls: Vec::new(),
    };
    calls.visit_program(&parsed.program);
    facts.decoder_calls = calls.calls;
    facts
}

struct SyntaxCollector<'a, 'b> {
    source: &'a str,
    budget: &'b mut AnalysisBudget,
    arrays: Vec<ArrayFact>,
    array_symbols: BTreeMap<SourceSpan, (String, SourceSpan)>,
}

impl<'a> Visit<'a> for SyntaxCollector<'_, '_> {
    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(identifier), Some(Expression::ArrayExpression(array))) =
            (&declarator.id, &declarator.init)
        {
            self.array_symbols.insert(
                source_span(array.span),
                (identifier.name.to_string(), source_span(identifier.span)),
            );
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_array_expression(&mut self, expression: &ArrayExpression<'a>) {
        if !self.budget.retain_array() {
            return;
        }
        let span = source_span(expression.span);
        let mut string_count = 0;
        let mut object_count = 0;
        let mut function_spans = Vec::new();
        for element in &expression.elements {
            let element_span = source_span(element.span());
            let text = self.slice(element_span).trim_start();
            if text.starts_with(['\'', '"']) {
                string_count += 1;
            }
            if text.starts_with('{') {
                object_count += 1;
            }
            if (text.starts_with("function") || looks_like_arrow(text)) && self.budget.retain_function() {
                function_spans.push(element_span);
            }
        }
        let (symbol, symbol_span) = self
            .array_symbols
            .get(&span)
            .map_or((None, None), |(name, name_span)| (Some(name.clone()), Some(*name_span)));
        self.arrays.push(ArrayFact {
            span,
            symbol,
            symbol_span,
            item_count: expression.elements.len(),
            string_count,
            object_count,
            function_spans,
        });
        if !self.budget.exhausted() {
            walk::walk_array_expression(self, expression);
        }
    }
}

impl SyntaxCollector<'_, '_> {
    fn slice(&self, span: SourceSpan) -> &str {
        self.source.get(span.start..span.end).unwrap_or_default()
    }
}

struct DecoderDefinitionCollector<'a, 'b, 'c> {
    source: &'a str,
    scoping: &'b Scoping,
    budget: &'c mut AnalysisBudget,
    decoders: Vec<DecoderFact>,
}

impl<'a> Visit<'a> for DecoderDefinitionCollector<'_, '_, '_> {
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        if self.budget.exhausted() {
            return;
        }
        if let Some(decoder) = recognize_decoder(self.source, self.scoping, function)
            && self.budget.retain_decoder()
        {
            self.decoders.push(decoder);
        }
        walk::walk_function(self, function, flags);
    }
}

struct DecoderCallCollector<'a, 'b> {
    scoping: &'a Scoping,
    budget: &'b mut AnalysisBudget,
    decoder_symbols: BTreeMap<SymbolId, String>,
    calls: Vec<DecoderCallFact>,
}

impl<'a> Visit<'a> for DecoderCallCollector<'_, '_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if self.budget.exhausted() {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            walk::walk_call_expression(self, call);
            return;
        };
        let Some(symbol_id) = reference_symbol(self.scoping, callee) else {
            walk::walk_call_expression(self, call);
            return;
        };
        let Some(decoder) = self.decoder_symbols.get(&symbol_id) else {
            walk::walk_call_expression(self, call);
            return;
        };
        let Some(Argument::StringLiteral(literal)) = call.arguments.first() else {
            walk::walk_call_expression(self, call);
            return;
        };
        if call.arguments.len() == 1 && self.budget.retain_encoded_literal(literal.value.len()) {
            self.calls.push(DecoderCallFact {
                decoder: decoder.clone(),
                span: source_span(call.span),
                literal: literal.value.to_string(),
            });
        }
        walk::walk_call_expression(self, call);
    }
}

fn recognize_decoder(source: &str, scoping: &Scoping, function: &Function<'_>) -> Option<DecoderFact> {
    let id = function.id.as_ref()?;
    let symbol_id = id.symbol_id.get()?;
    let body = function.body.as_ref()?;
    let rationale = if is_direct_wrapper(scoping, function, body) {
        "local decoder is a single-return wrapper with parameter-to-base64-primitive dataflow"
    } else if is_manual_decoder(source, scoping, function, body) {
        "local decoder has canonical alphabet, bitwise decode structure, buffer writes, and returns that buffer"
    } else {
        return None;
    };
    let span = source_span(id.span);
    if source.get(span.start..span.end)? != id.name.as_str() {
        return None;
    }
    Some(DecoderFact {
        name: id.name.to_string(),
        name_span: span,
        symbol_id,
        rationale: rationale.to_string(),
    })
}

fn is_direct_wrapper(scoping: &Scoping, function: &Function<'_>, body: &FunctionBody<'_>) -> bool {
    if body.statements.len() != 1 || function.params.items.len() != 1 {
        return false;
    }
    let BindingPattern::BindingIdentifier(parameter) = &function.params.items[0].pattern else {
        return false;
    };
    let Some(parameter_symbol) = parameter.symbol_id.get() else {
        return false;
    };
    let Statement::ReturnStatement(return_statement) = &body.statements[0] else {
        return false;
    };
    let Some(Expression::CallExpression(call)) = &return_statement.argument else {
        return false;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return false;
    };
    if callee.name != "atob" || reference_symbol(scoping, callee).is_some() || call.arguments.len() != 1 {
        return false;
    }
    matches!(
        call.arguments.first(),
        Some(Argument::Identifier(argument)) if reference_symbol(scoping, argument) == Some(parameter_symbol)
    )
}

fn is_manual_decoder(source: &str, scoping: &Scoping, function: &Function<'_>, body: &FunctionBody<'_>) -> bool {
    if function.params.items.len() != 1 {
        return false;
    }
    let BindingPattern::BindingIdentifier(parameter) = &function.params.items[0].pattern else {
        return false;
    };
    let Some(parameter_symbol) = parameter.symbol_id.get() else {
        return false;
    };
    let body_span = source_span(body.span);
    let Some(body_source) = source.get(body_span.start..body_span.end) else {
        return false;
    };
    let Some(wiring) = canonical_manual_wiring(body_source, parameter.name.as_str()) else {
        return false;
    };

    let mut shape = ManualDecoderShape {
        scoping,
        canonical_alphabet: false,
        operator_mask: 0,
        returned_symbols: BTreeSet::new(),
        member_write_symbols: BTreeMap::new(),
        allowed_calls: 0,
        has_forbidden_call: false,
        has_update_expression: false,
        has_unresolved_member_write: false,
    };
    shape.visit_function_body(body);
    if !shape.canonical_alphabet
        || shape.operator_mask != REQUIRED_MANUAL_OPERATORS
        || shape.has_forbidden_call
        || shape.has_update_expression
        || shape.has_unresolved_member_write
        || !matches!(shape.allowed_calls, 4 | 5)
        || shape.member_write_symbols.len() != 1
    {
        return false;
    }
    shape.returned_symbols.iter().any(|symbol| {
        *symbol != parameter_symbol
            && shape.member_write_symbols.get(symbol).copied() == Some(3)
            && body_source.contains(&format!("return {};", wiring.output_name))
    })
}

struct CanonicalManualWiring {
    output_name: String,
}

fn canonical_manual_wiring(body: &str, parameter: &str) -> Option<CanonicalManualWiring> {
    let alphabet_re = Regex::new(
        r#"(?:var|let|const)\s+([A-Za-z_$][\w$]*)\s*=\s*["']ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\+/["']\s*;"#,
    )
    .ok()?;
    let alphabet = alphabet_re.captures(body)?.get(1)?.as_str();
    let output_re =
        Regex::new(r"(?:var|let|const)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:\[\]|new\s+Uint8Array\([^;]+\))\s*;").ok()?;
    let output = output_re.captures(body)?.get(1)?.as_str();

    let index_re = Regex::new(
        r"(?:var|let|const)\s+([A-Za-z_$][\w$]*)\s*=\s*([A-Za-z_$][\w$]*)\.indexOf\(\s*([A-Za-z_$][\w$]*)\s*\[([^]]+)\]\s*\)\s*;",
    )
    .ok()?;
    let indices = index_re
        .captures_iter(body)
        .filter_map(|captures| {
            Some((
                captures.get(1)?.as_str().to_string(),
                captures.get(2)?.as_str().to_string(),
                captures.get(3)?.as_str().to_string(),
                compact(captures.get(4)?.as_str()),
            ))
        })
        .collect::<Vec<_>>();
    if indices.len() != 4
        || indices
            .iter()
            .any(|(_, table, input, _)| table != alphabet || input != parameter)
    {
        return None;
    }
    let input_index = indices[0].3.as_str();
    if !is_identifier(input_index)
        || indices[1].3 != format!("{input_index}+1")
        || indices[2].3 != format!("{input_index}+2")
        || indices[3].3 != format!("{input_index}+3")
    {
        return None;
    }

    let write_pattern = format!(r"{}\s*\[([^]]+)\]\s*=\s*([^;]+);", regex::escape(output));
    let write_re = Regex::new(&write_pattern).ok()?;
    let writes = write_re
        .captures_iter(body)
        .filter_map(|captures| Some((compact(captures.get(1)?.as_str()), compact(captures.get(2)?.as_str()))))
        .collect::<Vec<_>>();
    if writes.len() != 3 {
        return None;
    }
    let output_index = writes[0].0.as_str();
    if !is_identifier(output_index)
        || writes[1].0 != format!("{output_index}+1")
        || writes[2].0 != format!("{output_index}+2")
    {
        return None;
    }
    let names = indices.iter().map(|entry| entry.0.as_str()).collect::<Vec<_>>();
    let expected = [
        format!("{}<<2|{}>>4", names[0], names[1]),
        format!("{}&15<<4|{}>>2", names[1], names[2]),
        format!("{}&3<<6|{}", names[2], names[3]),
    ];
    if writes
        .iter()
        .zip(expected)
        .any(|((_, expression), expected)| strip_parentheses(expression) != expected)
    {
        return None;
    }

    Some(CanonicalManualWiring {
        output_name: output.to_string(),
    })
}

fn compact(text: &str) -> String {
    text.chars().filter(|character| !character.is_whitespace()).collect()
}

fn strip_parentheses(text: &str) -> String {
    compact(text).replace(['(', ')'], "")
}

fn is_identifier(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character == '$' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character == '$' || character.is_ascii_alphanumeric())
}

const OP_SHIFT_LEFT: u8 = 1;
const OP_SHIFT_RIGHT: u8 = 2;
const OP_BITWISE_AND: u8 = 4;
const OP_BITWISE_OR: u8 = 8;
const REQUIRED_MANUAL_OPERATORS: u8 = OP_SHIFT_LEFT | OP_SHIFT_RIGHT | OP_BITWISE_AND | OP_BITWISE_OR;

struct ManualDecoderShape<'a> {
    scoping: &'a Scoping,
    canonical_alphabet: bool,
    operator_mask: u8,
    returned_symbols: BTreeSet<SymbolId>,
    member_write_symbols: BTreeMap<SymbolId, usize>,
    allowed_calls: usize,
    has_forbidden_call: bool,
    has_update_expression: bool,
    has_unresolved_member_write: bool,
}

impl<'a> Visit<'a> for ManualDecoderShape<'_> {
    fn visit_function(&mut self, _function: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        if literal.value == "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/" {
            self.canonical_alphabet = true;
        }
    }

    fn visit_binary_expression(&mut self, expression: &BinaryExpression<'a>) {
        self.operator_mask |= match expression.operator {
            BinaryOperator::ShiftLeft => OP_SHIFT_LEFT,
            BinaryOperator::ShiftRight => OP_SHIFT_RIGHT,
            BinaryOperator::BitwiseAnd => OP_BITWISE_AND,
            BinaryOperator::BitwiseOR => OP_BITWISE_OR,
            _ => 0,
        };
        walk::walk_binary_expression(self, expression);
    }

    fn visit_return_statement(&mut self, statement: &ReturnStatement<'a>) {
        if let Some(Expression::Identifier(identifier)) = &statement.argument
            && let Some(symbol) = reference_symbol(self.scoping, identifier)
        {
            self.returned_symbols.insert(symbol);
        }
        walk::walk_return_statement(self, statement);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let allowed = match &call.callee {
            Expression::StaticMemberExpression(member) => {
                member.property.name == "indexOf"
                    || (member.property.name == "ceil"
                        && matches!(&member.object, Expression::Identifier(identifier) if identifier.name == "Math" && reference_symbol(self.scoping, identifier).is_none()))
            }
            _ => false,
        };
        if allowed {
            self.allowed_calls += 1;
        } else {
            self.has_forbidden_call = true;
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_update_expression(&mut self, expression: &oxc_ast::ast::UpdateExpression<'a>) {
        self.has_update_expression = true;
        walk::walk_update_expression(self, expression);
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        let object = match &expression.left {
            AssignmentTarget::ComputedMemberExpression(member) => Some(&member.object),
            AssignmentTarget::StaticMemberExpression(member) => Some(&member.object),
            AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                if reference_symbol(self.scoping, identifier).is_none() {
                    self.has_unresolved_member_write = true;
                }
                None
            }
            _ => None,
        };
        if let Some(Expression::Identifier(identifier)) = object {
            if let Some(symbol) = reference_symbol(self.scoping, identifier) {
                *self.member_write_symbols.entry(symbol).or_default() += 1;
            } else {
                self.has_unresolved_member_write = true;
            }
        }
        walk::walk_assignment_expression(self, expression);
    }
}

fn reference_symbol(scoping: &Scoping, identifier: &oxc_ast::ast::IdentifierReference<'_>) -> Option<SymbolId> {
    let reference_id = identifier.reference_id.get()?;
    scoping.get_reference(reference_id).symbol_id()
}

fn looks_like_arrow(text: &str) -> bool {
    text.get(..text.len().min(256))
        .is_some_and(|prefix| prefix.contains("=>"))
}

const fn source_span(span: oxc_span::Span) -> SourceSpan {
    SourceSpan {
        start: span.start as usize,
        end: span.end as usize,
    }
}
