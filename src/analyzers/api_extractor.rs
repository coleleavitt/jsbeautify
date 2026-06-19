//! API call and storage access extraction.
//!
//! Detects:
//!   - `fetch()` calls with URL arguments
//!   - `axios.get/post/put/delete/patch()` calls
//!   - `XMLHttpRequest.open("METHOD", "url")` calls
//!   - `localStorage.setItem/getItem()` and `sessionStorage.*`

use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, CallExpression, Expression};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::severity_for_kind;

pub const KIND_API_CALL: &str = "api_call";
pub const KIND_STORAGE_ACCESS: &str = "storage_access";

#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = ApiVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

struct ApiVisitor<'a> {
    source: &'a str,
    file_path: Option<&'a str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'a> ApiVisitor<'a> {
    fn push(&mut self, kind: &str, value: String, start: u32, end: u32) {
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

    fn extract_string_arg(&self, arg: &Argument<'a>) -> Option<String> {
        match arg {
            Argument::StringLiteral(s) => Some(s.value.to_string()),
            Argument::TemplateLiteral(t) => Some(t.span.source_text(self.source).to_string()),
            _ => None,
        }
    }
}

impl<'a> Visit<'a> for ApiVisitor<'a> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        match &call.callee {
            // fetch("url")
            Expression::Identifier(id) if id.name == "fetch" => {
                if let Some(arg) = call.arguments.first()
                    && let Some(val) = self.extract_string_arg(arg)
                {
                    self.push(KIND_API_CALL, val, call.span.start, call.span.end);
                }
            }
            Expression::StaticMemberExpression(sm) => {
                if let Expression::Identifier(obj) = &sm.object {
                    let obj_name = obj.name.as_str();
                    let prop = sm.property.name.as_str();

                    // axios.get/post/put/delete/patch("url")
                    if obj_name == "axios"
                        && matches!(
                            prop,
                            "get" | "post" | "put" | "delete" | "patch" | "request" | "head" | "options"
                        )
                        && let Some(arg) = call.arguments.first()
                        && let Some(val) = self.extract_string_arg(arg)
                    {
                        self.push(KIND_API_CALL, val, call.span.start, call.span.end);
                    }

                    // $.ajax, $.get, $.post
                    if (obj_name == "$" || obj_name == "jQuery")
                        && matches!(prop, "ajax" | "get" | "post" | "getJSON")
                        && let Some(arg) = call.arguments.first()
                        && let Some(val) = self.extract_string_arg(arg)
                    {
                        self.push(KIND_API_CALL, val, call.span.start, call.span.end);
                    }

                    // localStorage/sessionStorage.setItem/getItem/removeItem
                    if (obj_name == "localStorage" || obj_name == "sessionStorage")
                        && matches!(prop, "setItem" | "getItem" | "removeItem")
                        && let Some(arg) = call.arguments.first()
                        && let Some(val) = self.extract_string_arg(arg)
                    {
                        self.push(
                            KIND_STORAGE_ACCESS,
                            format!("{obj_name}.{prop}(\"{val}\")"),
                            call.span.start,
                            call.span.end,
                        );
                    }

                    // *.open("GET", "/api/...")  — XMLHttpRequest pattern
                    if prop == "open"
                        && call.arguments.len() >= 2
                        && let Some(Argument::StringLiteral(method)) = call.arguments.first()
                        && matches!(
                            method.value.as_str(),
                            "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS" | "HEAD"
                        )
                        && let Some(arg) = call.arguments.get(1)
                        && let Some(val) = self.extract_string_arg(arg)
                    {
                        self.push(KIND_API_CALL, val, call.span.start, call.span.end);
                    }
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fetch_calls() {
        let src = r#"fetch("/api/users"); fetch("/api/admin/config");"#;
        let m = analyze(src, None, FileType::Js);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|x| x.match_kind == "api_call"));
        assert!(m.iter().any(|x| x.match_value == "/api/users"));
        assert!(m.iter().any(|x| x.match_value == "/api/admin/config"));
    }

    #[test]
    fn finds_axios_calls() {
        let src = r#"axios.get("/api/profile"); axios.post("/api/login", data);"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.iter().any(|x| x.match_value == "/api/profile"));
        assert!(m.iter().any(|x| x.match_value == "/api/login"));
    }

    #[test]
    fn finds_storage_access() {
        let src = r#"localStorage.setItem("token", val); sessionStorage.getItem("session_id");"#;
        let m = analyze(src, None, FileType::Js);
        let storage_count = m.iter().filter(|x| x.match_kind == "storage_access").count();
        assert_eq!(storage_count, 2);
    }

    #[test]
    fn finds_xhr_open() {
        let src = r#"xhr.open("POST", "/api/internal/admin");"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.iter().any(|x| x.match_value == "/api/internal/admin"));
    }

    #[test]
    fn extracts_template_literal_argument_via_source_text() {
        // The template-literal branch of extract_string_arg uses
        // span.source_text(); cover it (incl. an interpolation) so the
        // raw-slice → source_text() change stays exercised.
        let src = "fetch(`/api/v2/users/${id}/profile`);";
        let m = analyze(src, None, FileType::Js);
        let call = m
            .iter()
            .find(|x| x.match_kind == "api_call")
            .expect("template-literal fetch should be captured");
        // source_text() returns the literal *including* the backticks and the
        // `${id}` interpolation, verbatim from the source.
        assert!(
            call.match_value.contains("/api/v2/users/"),
            "got {:?}",
            call.match_value
        );
        assert!(
            call.match_value.contains("${id}"),
            "interpolation retained: {:?}",
            call.match_value
        );
    }
}
