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

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, Expression, Function, VariableDeclarator,
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

    // Pass 0: map each named function to its parameter names, so a tainted
    // argument at a call site can taint the callee's parameter (inter-procedural).
    let mut params = FunctionParams { map: HashMap::new() };
    params.visit_program(&parse.program);

    // Pass 1: collect names of tainted bindings to a fixpoint. Each round runs
    // (a) intra-procedural propagation through assignments/decls, then
    // (b) inter-procedural propagation — a call `f(taintedArg)` taints f's
    // matching parameter — so the next round propagates inside f's body.
    let mut tainted: HashSet<String> = HashSet::new();
    for _ in 0..8 {
        let before = tainted.len();
        let mut collector = TaintCollector { tainted: &mut tainted };
        collector.visit_program(&parse.program);
        let mut propagator = CallPropagator {
            params: &params.map,
            tainted: &mut tainted,
        };
        propagator.visit_program(&parse.program);
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

/// Pass 0 — map each *named* function to its ordered parameter names, so a
/// tainted call argument can taint the matching callee parameter
/// (inter-procedural taint).
#[derive(Default)]
struct FunctionParams {
    /// function name → ordered parameter identifier names.
    map: HashMap<String, Vec<String>>,
}

impl FunctionParams {
    fn record(&mut self, name: &str, params: &oxc_ast::ast::FormalParameters<'_>) {
        let names: Vec<String> = params
            .items
            .iter()
            .filter_map(|p| match &p.pattern {
                BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
                _ => None,
            })
            .collect();
        if !names.is_empty() {
            self.map.insert(name.to_string(), names);
        }
    }
}

impl<'a> Visit<'a> for FunctionParams {
    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        if let Some(id) = &func.id {
            self.record(id.name.as_str(), &func.params);
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        // `const f = (a, b) => …` / `const f = function (a, b) {}`.
        if let BindingPattern::BindingIdentifier(id) = &decl.id {
            match &decl.init {
                Some(Expression::ArrowFunctionExpression(arrow)) => self.record(id.name.as_str(), &arrow.params),
                Some(Expression::FunctionExpression(f)) => self.record(id.name.as_str(), &f.params),
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, decl);
    }
}

/// Inter-procedural pass — for every call `f(args…)` where an argument is
/// tainted, taint `f`'s corresponding parameter name so the next fixpoint round
/// propagates taint through `f`'s body.
struct CallPropagator<'a, 'p> {
    params: &'p HashMap<String, Vec<String>>,
    tainted: &'a mut HashSet<String>,
}

impl<'a> Visit<'a> for CallPropagator<'a, '_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // Only resolve a plain identifier callee `f(...)`. A method call
        // `obj.f(...)` is NOT matched by the bare method name: without scope
        // resolution we can't know it refers to a standalone `function f`, and
        // assuming so produces false positives (e.g. tainting an unrelated
        // top-level `render` because some object also has a `render` method).
        if let Expression::Identifier(callee) = &call.callee
            && let Some(param_names) = self.params.get(callee.name.as_str())
        {
            for (i, arg) in call.arguments.iter().enumerate() {
                if let Some(expr) = arg.as_expression()
                    && expr_is_tainted(expr, self.tainted)
                    && let Some(pname) = param_names.get(i)
                {
                    self.tainted.insert(pname.clone());
                }
            }
        }
        walk::walk_call_expression(self, call);
    }
}

/// Intra-procedural pass — collects tainted binding names.
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
    fn flags_ssrf_fetch_of_tainted() {
        let src = "var u = location.search; fetch(u);";
        assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn flags_ssrf_axios_get() {
        let src = "var u = location.hash; axios.get(u);";
        assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn flags_ssrf_new_url_request() {
        let src = "var u = document.URL; new Request(u);";
        assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn ssrf_ignores_map_get() {
        // `map.get(tainted)` is a Map lookup, not an HTTP request — must not flag.
        let src = "var k = location.hash; myMap.get(k);";
        assert!(!kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn flags_ssrf_interprocedural() {
        // Tainted URL flows through a wrapper into fetch in a different function.
        let src = "
            function load(url) { return fetch(url); }
            var u = location.search;
            load(u);
        ";
        assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
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

    #[test]
    fn interprocedural_source_in_caller_sink_in_callee() {
        // Source is in the caller; the sink lives in a separate function reached
        // via the tainted argument. Intra-procedural taint would miss this.
        let src = "
            function render(html) { document.body.innerHTML = html; }
            var u = location.hash;
            render(u);
        ";
        assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn interprocedural_arrow_function() {
        let src = "
            const go = (target) => { window.location.href = target; };
            var dest = location.search;
            go(dest);
        ";
        assert!(kinds(src).contains(&"open_redirect".to_string()), "{:?}", kinds(src));
    }

    #[test]
    fn interprocedural_does_not_taint_unrelated_param() {
        // Only the tainted-position parameter becomes tainted, not a sibling.
        let src = "
            function sink(safe, danger) { el.innerHTML = safe; }
            var t = location.hash;
            sink('literal', t);
        ";
        // `safe` (position 0) got a literal; only `danger` (position 1) is tainted,
        // and it never reaches the sink — so no finding.
        assert!(kinds(src).is_empty(), "{:?}", kinds(src));
    }

    #[test]
    fn method_call_does_not_taint_standalone_function() {
        // This test demonstrates the bug: calling a method with a tainted argument
        // should NOT taint a standalone function with the same name.
        let src = "
            function render(html) {
                document.body.innerHTML = html;
            }

            const obj = {
                render: function(data) {
                    console.log(data);
                }
            };

            var userInput = location.hash;
            obj.render(userInput);
        ";

        let result = kinds(src);
        // Should be empty — no dom_xss finding because standalone render() is never called
        assert!(
            result.is_empty(),
            "False positive: tainted method call triggered standalone function's sink: {:?}",
            result
        );
    }
}
