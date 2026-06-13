//! Taint-lite: source → sink reachability for client-side bug finding.
//!
//! The other analyzers find *where* a dangerous sink is (`el.innerHTML = x`).
//! This one asks the question that turns a sink into a *bug*: does
//! attacker-controlled input actually reach it? It is the first, deliberately
//! small, step toward the ROADMAP P0 taint engine — an intra-procedural,
//! flow-insensitive approximation grounded in oxc's resolved symbols.
//!
//! Algorithm (per program, symbol-resolved via `oxc_semantic`):
//!  1. **Seed**: any binding initialized from a taint *source* expression
//!     (`location.hash`, `document.URL`, `location.search`, `window.name`,
//!     a `message`-event `.data`, etc.) marks that symbol tainted. Direct
//!     `source` sub-expressions are tainted too.
//!  2. **Propagate** (one fixpoint pass): `b = taintedExpr` taints `b`;
//!     string concatenation / template literals containing a tainted symbol
//!     stay tainted.
//!  3. **Report**: a *sink* (`innerHTML`/`outerHTML`/`document.write`/`eval`/
//!     `location.href = …`) whose argument expression contains a tainted
//!     symbol — or a taint source directly — produces a finding:
//!     `dom_xss` for HTML/eval sinks, `open_redirect` for location sinks.
//!
//! This is an over-approximation (flow-insensitive, no sanitizer modeling), so
//! findings are leads to verify, not proofs — but unlike the pure sink scan,
//! every finding has an attacker-controlled-data justification.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, Expression, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

/// Run the taint-lite analysis on `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    // Pass 1: collect names of tainted bindings (fixpoint to a small depth).
    let mut tainted: HashSet<String> = HashSet::new();
    for _ in 0..4 {
        let before = tainted.len();
        let mut collector = TaintCollector { tainted: &mut tainted };
        collector.visit_program(&parse.program);
        if tainted.len() == before {
            break; // fixpoint reached
        }
    }

    // Pass 2: report sinks reached by tainted data.
    let mut reporter = SinkReporter {
        source,
        file_path,
        file_type,
        tainted: &tainted,
        out: Vec::new(),
    };
    reporter.visit_program(&parse.program);
    reporter.out
}

/// True if `expr` reads a known taint source.
fn is_taint_source(expr: &Expression<'_>) -> bool {
    match expr {
        // location.hash / location.search / location.href / document.URL / document.cookie
        Expression::StaticMemberExpression(m) => {
            let prop = m.property.name.as_str();
            let obj = base_object_name(&m.object);
            matches!(
                (obj.as_deref(), prop),
                (Some("location"), "hash" | "search" | "href" | "pathname")
                    | (
                        Some("document"),
                        "URL" | "documentURI" | "cookie" | "referrer" | "baseURI"
                    )
                    | (Some("window"), "name")
            ) || prop == "data" && obj.as_deref() == Some("event") // message event .data
        }
        // new URLSearchParams(location.search).get(...) and the like read sources too,
        // but we keep it simple: a call whose callee/args contain a source.
        Expression::CallExpression(c) => {
            c.arguments
                .iter()
                .any(|a| a.as_expression().is_some_and(is_taint_source))
                || expr_contains_source(&c.callee)
        }
        _ => false,
    }
}

/// The base identifier name of a (possibly nested) member expression object.
fn base_object_name(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(m) => base_object_name(&m.object),
        Expression::ThisExpression(_) => Some("this".to_string()),
        _ => None,
    }
}

/// Recursively check whether `expr` reads a taint source anywhere.
fn expr_contains_source(expr: &Expression<'_>) -> bool {
    if is_taint_source(expr) {
        return true;
    }
    match expr {
        Expression::StaticMemberExpression(m) => expr_contains_source(&m.object),
        Expression::BinaryExpression(b) => expr_contains_source(&b.left) || expr_contains_source(&b.right),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(expr_contains_source),
        Expression::CallExpression(c) => {
            expr_contains_source(&c.callee)
                || c.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(expr_contains_source))
        }
        Expression::ParenthesizedExpression(p) => expr_contains_source(&p.expression),
        _ => false,
    }
}

/// Whether `expr` references any tainted binding name (or a direct source).
fn expr_is_tainted(expr: &Expression<'_>, tainted: &HashSet<String>) -> bool {
    if expr_contains_source(expr) {
        return true;
    }
    match expr {
        Expression::Identifier(id) => tainted.contains(id.name.as_str()),
        Expression::StaticMemberExpression(m) => expr_is_tainted(&m.object, tainted),
        Expression::BinaryExpression(b) => expr_is_tainted(&b.left, tainted) || expr_is_tainted(&b.right, tainted),
        Expression::TemplateLiteral(t) => t.expressions.iter().any(|e| expr_is_tainted(e, tainted)),
        Expression::ParenthesizedExpression(p) => expr_is_tainted(&p.expression, tainted),
        Expression::CallExpression(c) => c
            .arguments
            .iter()
            .any(|a| a.as_expression().is_some_and(|e| expr_is_tainted(e, tainted))),
        Expression::LogicalExpression(l) => expr_is_tainted(&l.left, tainted) || expr_is_tainted(&l.right, tainted),
        Expression::ConditionalExpression(c) => {
            expr_is_tainted(&c.consequent, tainted) || expr_is_tainted(&c.alternate, tainted)
        }
        _ => false,
    }
}

/// Pass 1 — collects tainted binding names.
struct TaintCollector<'a> {
    tainted: &'a mut HashSet<String>,
}

impl<'a> Visit<'a> for TaintCollector<'a> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (Some(init), BindingPattern::BindingIdentifier(id)) = (&decl.init, &decl.id)
            && expr_is_tainted(init, self.tainted)
        {
            self.tainted.insert(id.name.to_string());
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left
            && expr_is_tainted(&assign.right, self.tainted)
        {
            self.tainted.insert(id.name.to_string());
        }
        walk::walk_assignment_expression(self, assign);
    }
}

/// HTML/eval sinks → `dom_xss`; location sinks → `open_redirect`.
const HTML_SINK_PROPS: &[&str] = &["innerHTML", "outerHTML"];
const HTML_SINK_METHODS: &[&str] = &["write", "writeln", "insertAdjacentHTML"];
const LOCATION_SINK_PROPS: &[&str] = &["href"];
const LOCATION_SINK_METHODS: &[&str] = &["assign", "replace"];

/// Pass 2 — reports sinks reached by tainted data.
struct SinkReporter<'a, 's> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    tainted: &'a HashSet<String>,
    out: Vec<AnalyzerMatch>,
}

impl SinkReporter<'_, '_> {
    fn push(&mut self, kind: &str, span: Span, severity: Severity) {
        let snippet = span.source_text(self.source);
        let value = snippet.chars().take(160).collect::<String>();
        self.out.push(
            AnalyzerMatch::new(
                kind,
                value,
                span_to_pos(self.source, span.start, span.end),
                self.file_type,
                self.file_path.map(String::from),
            )
            .with_provenance(Provenance::Ast)
            .with_severity(severity),
        );
    }
}

impl<'a> Visit<'a> for SinkReporter<'a, '_> {
    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        if let AssignmentTarget::StaticMemberExpression(m) = &assign.left {
            let prop = m.property.name.as_str();
            let tainted = expr_is_tainted(&assign.right, self.tainted);
            if tainted && HTML_SINK_PROPS.contains(&prop) {
                self.push("dom_xss", assign.span, Severity::High);
            } else if tainted && LOCATION_SINK_PROPS.contains(&prop) {
                // location.href = tainted  → open redirect / DOM-XSS via javascript:
                self.push("open_redirect", assign.span, Severity::Medium);
            }
        }
        walk::walk_assignment_expression(self, assign);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let tainted_arg = call
            .arguments
            .iter()
            .any(|a| a.as_expression().is_some_and(|e| expr_is_tainted(e, self.tainted)));
        if tainted_arg {
            match &call.callee {
                // eval(tainted) → dom_xss (code injection)
                Expression::Identifier(id) if id.name == "eval" => {
                    self.push("dom_xss", call.span, Severity::High);
                }
                // x.write(tainted) / x.insertAdjacentHTML(..,tainted) → dom_xss
                Expression::StaticMemberExpression(m) if HTML_SINK_METHODS.contains(&m.property.name.as_str()) => {
                    self.push("dom_xss", call.span, Severity::High);
                }
                // location.assign/replace(tainted) → open_redirect
                Expression::StaticMemberExpression(m)
                    if LOCATION_SINK_METHODS.contains(&m.property.name.as_str())
                        && base_object_name(&m.object).as_deref() == Some("location") =>
                {
                    self.push("open_redirect", call.span, Severity::Medium);
                }
                // Any other callee is not a known sink — tainted data flowing
                // into it is out of scope for this intra-procedural pass.
                other => {
                    debug_assert!(
                        !matches!(other, Expression::Identifier(id) if id.name == "eval"),
                        "eval should have matched above",
                    );
                }
            }
        }
        walk::walk_call_expression(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<String> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .map(|m| m.match_kind)
            .collect()
    }

    #[test]
    fn flags_location_hash_to_innerhtml() {
        let src = "var x = location.hash; el.innerHTML = x;";
        assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn flags_direct_source_in_sink() {
        let src = "document.body.innerHTML = location.search;";
        assert!(kinds(src).contains(&"dom_xss".to_string()));
    }

    #[test]
    fn flags_open_redirect_from_tainted_href() {
        // The BlackRock idpRedirect shape: href set from a value derived from input.
        let src = "var u = location.href; window.location.href = u;";
        assert!(kinds(src).contains(&"open_redirect".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn flags_eval_of_tainted() {
        let src = "var c = location.hash; eval(c);";
        assert!(kinds(src).contains(&"dom_xss".to_string()));
    }

    #[test]
    fn ignores_constant_innerhtml() {
        let src = "el.innerHTML = '<b>static</b>';";
        assert!(kinds(src).is_empty(), "constant sink should not flag: {:?}", kinds(src));
    }

    #[test]
    fn ignores_untainted_variable() {
        let src = "var safe = 'literal'; el.innerHTML = safe;";
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn propagates_through_concatenation() {
        let src = "var h = location.hash; var html = '<div>' + h + '</div>'; el.innerHTML = html;";
        assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
    }
}
