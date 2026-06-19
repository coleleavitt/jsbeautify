use oxc_ast::ast::{
    Argument, AssignmentExpression, AssignmentTarget, CallExpression, Expression, JSXAttribute, JSXAttributeName,
    NewExpression,
};
use oxc_ast_visit::{Visit, walk};

use super::super::ast_walker::span_to_pos;
use super::super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::super::regex_bank::severity_for_kind;
use super::kinds;

/// HTML-manipulation property names that count as a write sink.
///
/// `el.innerHTML = X` / `el.outerHTML = X` / `el.insertAdjacentHTML(...)` etc.
const HTML_WRITE_PROPS: &[&str] = &[
    "innerHTML",
    "outerHTML",
    // also: setHTML / setHTMLUnsafe (newer API)
    "setHTML",
    "setHTMLUnsafe",
];
const HTML_WRITE_METHODS: &[&str] = &["insertAdjacentHTML", "writeln", "write", "createContextualFragment"];

/// Location-sink property names.
const LOCATION_WRITE_PROPS: &[&str] = &["href", "hash"];
const LOCATION_WRITE_METHODS: &[&str] = &["assign", "replace"];

pub(super) struct SinkVisitor<'s> {
    pub(super) source: &'s str,
    pub(super) file_path: Option<&'s str>,
    pub(super) file_type: FileType,
    pub(super) out: Vec<AnalyzerMatch>,
}

impl SinkVisitor<'_> {
    fn emit(&mut self, kind: &'static str, value: String, start: u32, end: u32) {
        self.out.push(AnalyzerMatch {
            match_kind: kind.to_string(),
            match_value: value,
            position: span_to_pos(self.source, start, end),
            file_type: self.file_type,
            file_path: self.file_path.map(str::to_string),
            severity: severity_for_kind(kind),
            provenance: Provenance::Ast,
            metadata: None,
        });
    }

    fn snippet(&self, start: u32, end: u32) -> String {
        let s = start as usize;
        let e = (end as usize).min(self.source.len());
        if s > e || s >= self.source.len() {
            return String::new();
        }
        let max_len = 200;
        let raw = &self.source[s..e];
        if raw.len() > max_len {
            format!("{}…", &raw[..max_len])
        } else {
            raw.to_string()
        }
    }

    /// `target.location` or `window.location` or just bare `location`?
    fn is_location_object(expr: &Expression<'_>) -> bool {
        match expr {
            Expression::Identifier(id) => id.name == "location",
            Expression::StaticMemberExpression(m) => m.property.name == "location",
            Expression::ComputedMemberExpression(m) => {
                if let Expression::StringLiteral(s) = &m.expression {
                    s.value == "location"
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl<'a> Visit<'a> for SinkVisitor<'_> {
    fn visit_assignment_expression(&mut self, expr: &AssignmentExpression<'a>) {
        // X.innerHTML = ... | X.outerHTML = ... | X.location = "..." | X.href = "..."
        if let AssignmentTarget::StaticMemberExpression(sm) = &expr.left {
            let prop = sm.property.name.as_str();

            if HTML_WRITE_PROPS.contains(&prop) {
                self.emit(
                    kinds::HTML_MANIPULATION,
                    self.snippet(expr.span.start, expr.span.end),
                    expr.span.start,
                    expr.span.end,
                );
            }

            if LOCATION_WRITE_PROPS.contains(&prop) {
                // Heuristic: only count if the object is window.location / document.location / similar.
                if Self::is_location_object(&sm.object)
                    || matches!(&sm.object, Expression::Identifier(id) if id.name == "location")
                {
                    self.emit(
                        kinds::LOCATION_ASSIGNMENT,
                        self.snippet(expr.span.start, expr.span.end),
                        expr.span.start,
                        expr.span.end,
                    );
                }
            }

            // window.onmessage = handler
            if prop == "onmessage" {
                self.emit(
                    kinds::WINDOW_ONMESSAGE,
                    self.snippet(expr.span.start, expr.span.end),
                    expr.span.start,
                    expr.span.end,
                );
            }

            // window.location = "..."  (whole-object replacement)
            if prop == "location" {
                self.emit(
                    kinds::LOCATION_ASSIGNMENT,
                    self.snippet(expr.span.start, expr.span.end),
                    expr.span.start,
                    expr.span.end,
                );
            }
        }

        walk::walk_assignment_expression(self, expr);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // Look at member-expression callees: foo.bar(...)
        if let Expression::StaticMemberExpression(sm) = &call.callee {
            let method = sm.property.name.as_str();

            // location.assign(...) / location.replace(...)
            if Self::is_location_object(&sm.object) && LOCATION_WRITE_METHODS.contains(&method) {
                self.emit(
                    kinds::LOCATION_ASSIGNMENT,
                    self.snippet(call.span.start, call.span.end),
                    call.span.start,
                    call.span.end,
                );
            }

            // foo.postMessage(...)
            if method == "postMessage" {
                self.emit(
                    kinds::POST_MESSAGE,
                    self.snippet(call.span.start, call.span.end),
                    call.span.start,
                    call.span.end,
                );
            }

            // X.insertAdjacentHTML / .write / .writeln / .createContextualFragment
            if HTML_WRITE_METHODS.contains(&method) {
                self.emit(
                    kinds::HTML_MANIPULATION,
                    self.snippet(call.span.start, call.span.end),
                    call.span.start,
                    call.span.end,
                );
            }

            // addEventListener('message', handler)  or  ('message', h, opts)
            if method == "addEventListener"
                && let Some(Argument::StringLiteral(first)) = call.arguments.first()
                && first.value == "message"
            {
                self.emit(
                    kinds::WINDOW_ONMESSAGE,
                    self.snippet(call.span.start, call.span.end),
                    call.span.start,
                    call.span.end,
                );
            }
        }

        // new URLSearchParams(...)
        if let Expression::Identifier(id) = &call.callee
            && id.name == "URLSearchParams"
        {
            self.emit(
                kinds::URL_SEARCH_PARAMS,
                self.snippet(call.span.start, call.span.end),
                call.span.start,
                call.span.end,
            );
        }

        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, expr: &NewExpression<'a>) {
        if let Expression::Identifier(id) = &expr.callee
            && id.name == "URLSearchParams"
        {
            self.emit(
                kinds::URL_SEARCH_PARAMS,
                self.snippet(expr.span.start, expr.span.end),
                expr.span.start,
                expr.span.end,
            );
        }
        walk::walk_new_expression(self, expr);
    }

    fn visit_jsx_attribute(&mut self, attr: &JSXAttribute<'a>) {
        if let JSXAttributeName::Identifier(name) = &attr.name
            && name.name == "dangerouslySetInnerHTML"
        {
            self.emit(
                kinds::DANGEROUSLY_SET_INNER_HTML,
                self.snippet(attr.span.start, attr.span.end),
                attr.span.start,
                attr.span.end,
            );
        }
        walk::walk_jsx_attribute(self, attr);
    }
}
