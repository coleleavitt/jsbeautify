use std::collections::HashSet;

use oxc_ast::ast::{AssignmentExpression, AssignmentTarget, CallExpression, Expression};
use oxc_ast_visit::{Visit, walk};
use oxc_span::Span;

use super::super::ast_walker::span_to_pos;
use super::super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::super::regex_bank::Severity;
use super::expr::{base_object_name, expr_is_tainted};

/// HTML/eval sinks → `dom_xss`; location sinks → `open_redirect`.
const HTML_SINK_PROPS: &[&str] = &["innerHTML", "outerHTML"];
const HTML_SINK_METHODS: &[&str] = &["write", "writeln", "insertAdjacentHTML"];
const LOCATION_SINK_PROPS: &[&str] = &["href"];
const LOCATION_SINK_METHODS: &[&str] = &["assign", "replace"];

/// SSRF sinks (CWE-918): functions that issue a request to a URL. A tainted URL
/// reaching one means attacker-controlled-server-side/cross-origin fetch.
/// Plain-identifier callees (`fetch(tainted)`).
const SSRF_FNS: &[&str] = &["fetch"];
/// HTTP-client method names (`axios.get(tainted)`, `xhr.open(m, tainted)`, …).
const SSRF_METHODS: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "request",
    "ajax",
    "open",
    "sendBeacon",
];
/// Object base-names that denote an HTTP client (so `obj.get(...)` is a request,
/// not e.g. `map.get(...)`).
const HTTP_CLIENT_OBJECTS: &[&str] = &[
    "axios",
    "$",
    "http",
    "https",
    "xhr",
    "client",
    "request",
    "got",
    "superagent",
    "fetch",
    "navigator",
    "jquery",
];
/// Constructors that take a URL (`new URL/Request/WebSocket/EventSource(tainted)`).
const SSRF_CTORS: &[&str] = &["URL", "Request", "WebSocket", "EventSource"];

/// Pass 2 — reports sinks reached by tainted data.
pub(super) struct SinkReporter<'a, 's> {
    pub(super) source: &'s str,
    pub(super) file_path: Option<&'s str>,
    pub(super) file_type: FileType,
    pub(super) tainted: &'a HashSet<String>,
    pub(super) out: Vec<AnalyzerMatch>,
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
                // fetch(tainted) → ssrf
                Expression::Identifier(id) if SSRF_FNS.contains(&id.name.as_str()) => {
                    self.push("ssrf", call.span, Severity::High);
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
                // axios.get(tainted) / xhr.open(m, tainted) / navigator.sendBeacon(tainted) → ssrf.
                // Gated on an HTTP-client object name so `map.get(tainted)` doesn't flag.
                Expression::StaticMemberExpression(m)
                    if SSRF_METHODS.contains(&m.property.name.as_str())
                        && base_object_name(&m.object)
                            .as_deref()
                            .is_some_and(|o| HTTP_CLIENT_OBJECTS.contains(&o)) =>
                {
                    self.push("ssrf", call.span, Severity::High);
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

    fn visit_new_expression(&mut self, new: &oxc_ast::ast::NewExpression<'a>) {
        // new URL/Request/WebSocket/EventSource(tainted) → ssrf.
        if let Expression::Identifier(id) = &new.callee
            && SSRF_CTORS.contains(&id.name.as_str())
            && new
                .arguments
                .iter()
                .any(|a| a.as_expression().is_some_and(|e| expr_is_tainted(e, self.tainted)))
        {
            self.push("ssrf", new.span, Severity::High);
        }
        walk::walk_new_expression(self, new);
    }
}
