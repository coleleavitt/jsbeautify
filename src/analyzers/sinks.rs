//! Dangerous-sink detection: HTML manipulation, location assignment,
//! postMessage, onmessage, dangerouslySetInnerHTML, URLSearchParams.
//!
//! Ports the following jxscout analyzers in one consolidated module:
//! - `js_ast_html_manipulation_analyzer.rs`         → `html_manipulation`
//! - `js_ast_location_analyzer.rs`                  → `location_assignment`
//! - `js_ast_onmessage_analyzer.rs`                 → `window_onmessage`
//! - `js_ast_postmessage_analyzer.rs`               → `post_message`
//! - `js_ast_react_dangerously_set_inner_analyzer.rs` → `dangerously_set_inner_html`
//! - `js_ast_url_search_params_analyzer.rs`         → `url_search_params`
//!
//! Output schema is stable: `match_kind` aligns 1-to-1 with jxscout's strings.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType};
use super::regex_bank::severity_for_kind;

/// Sink kinds emitted by [`analyze`].
pub mod kinds {
    pub const HTML_MANIPULATION: &str = "html_manipulation";
    pub const LOCATION_ASSIGNMENT: &str = "location_assignment";
    pub const WINDOW_ONMESSAGE: &str = "window_onmessage";
    pub const POST_MESSAGE: &str = "post_message";
    pub const DANGEROUSLY_SET_INNER_HTML: &str = "dangerously_set_inner_html";
    pub const URL_SEARCH_PARAMS: &str = "url_search_params";
}

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

/// Run all sink analyzers on `source` in a single AST walk.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = SinkVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

struct SinkVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'s> SinkVisitor<'s> {
    fn emit(&mut self, kind: &'static str, value: String, start: u32, end: u32) {
        self.out.push(AnalyzerMatch {
            match_kind: kind.to_string(),
            match_value: value,
            position: span_to_pos(self.source, start, end),
            file_type: self.file_type,
            file_path: self.file_path.map(str::to_string),
            severity: severity_for_kind(kind),
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

    /// Get the static name from a [`StaticMemberExpression`]'s property
    /// or a [`ComputedMemberExpression`]'s string literal key.
    fn static_member_name<'a>(member: &MemberExpression<'a>) -> Option<&'a str> {
        match member {
            MemberExpression::StaticMemberExpression(sm) => Some(sm.property.name.as_str()),
            MemberExpression::ComputedMemberExpression(cm) => {
                if let Expression::StringLiteral(s) = &cm.expression {
                    Some(s.value.as_str())
                } else {
                    None
                }
            }
            MemberExpression::PrivateFieldExpression(_) => None,
        }
    }

    /// `target.location` or `window.location` or just bare `location`?
    fn is_location_object<'a>(expr: &Expression<'a>) -> bool {
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

impl<'a, 's> Visit<'a> for SinkVisitor<'s> {
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
            if method == "addEventListener" {
                if let Some(Argument::StringLiteral(first)) = call.arguments.first() {
                    if first.value == "message" {
                        self.emit(
                            kinds::WINDOW_ONMESSAGE,
                            self.snippet(call.span.start, call.span.end),
                            call.span.start,
                            call.span.end,
                        );
                    }
                }
            }
        }

        // new URLSearchParams(...)
        if let Expression::Identifier(id) = &call.callee {
            if id.name == "URLSearchParams" {
                self.emit(
                    kinds::URL_SEARCH_PARAMS,
                    self.snippet(call.span.start, call.span.end),
                    call.span.start,
                    call.span.end,
                );
            }
        }

        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, expr: &NewExpression<'a>) {
        if let Expression::Identifier(id) = &expr.callee {
            if id.name == "URLSearchParams" {
                self.emit(
                    kinds::URL_SEARCH_PARAMS,
                    self.snippet(expr.span.start, expr.span.end),
                    expr.span.start,
                    expr.span.end,
                );
            }
        }
        walk::walk_new_expression(self, expr);
    }

    fn visit_jsx_attribute(&mut self, attr: &JSXAttribute<'a>) {
        if let JSXAttributeName::Identifier(name) = &attr.name {
            if name.name == "dangerouslySetInnerHTML" {
                self.emit(
                    kinds::DANGEROUSLY_SET_INNER_HTML,
                    self.snippet(attr.span.start, attr.span.end),
                    attr.span.start,
                    attr.span.end,
                );
            }
        }
        walk::walk_jsx_attribute(self, attr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_for(src: &str) -> Vec<String> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .map(|m| m.match_kind)
            .collect()
    }

    #[test]
    fn finds_inner_html_assignment() {
        assert!(kinds_for("el.innerHTML = userInput;").contains(&kinds::HTML_MANIPULATION.to_string()));
        assert!(kinds_for("foo.outerHTML = x;").contains(&kinds::HTML_MANIPULATION.to_string()));
    }

    #[test]
    fn finds_insert_adjacent_html() {
        assert!(kinds_for("el.insertAdjacentHTML('beforeend', x);").contains(&kinds::HTML_MANIPULATION.to_string()));
    }

    #[test]
    fn finds_document_write() {
        assert!(kinds_for("document.write(x);").contains(&kinds::HTML_MANIPULATION.to_string()));
    }

    #[test]
    fn finds_location_href_assignment() {
        let k = kinds_for("location.href = userInput;");
        assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_location_assign_call() {
        let k = kinds_for("location.assign('https://evil.com');");
        assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_location_replace_call() {
        let k = kinds_for("window.location.replace('x');");
        assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_postmessage_call() {
        let k = kinds_for("frame.postMessage(data, '*');");
        assert!(k.contains(&kinds::POST_MESSAGE.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_window_onmessage() {
        let k = kinds_for("window.onmessage = handler;");
        assert!(k.contains(&kinds::WINDOW_ONMESSAGE.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_addeventlistener_message() {
        let k = kinds_for("window.addEventListener('message', handler);");
        assert!(k.contains(&kinds::WINDOW_ONMESSAGE.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_dangerously_set_inner_html() {
        let src = r#"const x = <div dangerouslySetInnerHTML={{__html: y}} />;"#;
        let k = kinds_for(src);
        assert!(k.contains(&kinds::DANGEROUSLY_SET_INNER_HTML.to_string()), "got {k:?}");
    }

    #[test]
    fn finds_url_search_params() {
        let k = kinds_for("new URLSearchParams(window.location.search);");
        assert!(k.contains(&kinds::URL_SEARCH_PARAMS.to_string()), "got {k:?}");
    }

    #[test]
    fn no_false_positive_on_unrelated() {
        let k = kinds_for("const x = 1; function foo() { return x + 2; }");
        assert!(k.is_empty(), "got {k:?}");
    }
}
