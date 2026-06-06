//! GraphQL operation extraction.
//!
//! Ports `crates/core/analyzers/src/js_ast_graphql_analyzer.rs`.
//!
//! Looks for string literals & template literals containing `query`,
//! `mutation`, or `fragment` keywords with a `{` body, plus calls like
//! `gql\`...\`` (Apollo / urql tagged template).

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::helpers;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

/// Extract GraphQL operations from `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = GqlVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

/// Parse the leading GraphQL operation/fragment header into structured metadata
/// (`operation_type`, `operation_name`, `fragment_name`, `on_type`) so consumers
/// get an indexed operation catalog rather than just raw SDL.
fn gql_metadata(body: &str) -> serde_json::Value {
    lazy_static::lazy_static! {
        static ref OP_RE: regex::Regex =
            regex::Regex::new(r"(?m)^\s*(query|mutation|subscription)\s+([A-Za-z_]\w*)").unwrap();
        static ref FRAG_RE: regex::Regex =
            regex::Regex::new(r"(?m)^\s*fragment\s+([A-Za-z_]\w*)\s+on\s+([A-Za-z_]\w*)").unwrap();
        static ref OP_ANON_RE: regex::Regex =
            regex::Regex::new(r"(?m)^\s*(query|mutation|subscription)\b").unwrap();
    }
    let mut op_type: Option<String> = None;
    let mut op_name: Option<String> = None;
    let mut frag_name: Option<String> = None;
    let mut on_type: Option<String> = None;
    if let Some(c) = OP_RE.captures(body) {
        op_type = Some(c[1].to_string());
        op_name = Some(c[2].to_string());
    } else if let Some(c) = FRAG_RE.captures(body) {
        op_type = Some("fragment".to_string());
        frag_name = Some(c[1].to_string());
        on_type = Some(c[2].to_string());
    } else if let Some(c) = OP_ANON_RE.captures(body) {
        op_type = Some(c[1].to_string());
    }
    serde_json::json!({
        "operation_type": op_type,
        "operation_name": op_name,
        "fragment_name": frag_name,
        "on_type": on_type,
    })
}

struct GqlVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'s> GqlVisitor<'s> {
    fn maybe_emit(&mut self, body: &str, start: u32, end: u32) {
        // Heuristic: must start with a GraphQL keyword (case-sensitive — GraphQL is case-sensitive)
        // AND have a `{` somewhere.
        let trimmed = body.trim_start();
        if !(trimmed.starts_with("query")
            || trimmed.starts_with("mutation")
            || trimmed.starts_with("fragment")
            || trimmed.starts_with("subscription"))
        {
            return;
        }
        if !body.contains('{') {
            return;
        }

        // Tighter regex check
        if !helpers::GRAPHQL_REGEX.is_match(body) {
            return;
        }

        self.out.push(AnalyzerMatch {
            match_kind: "graphql".to_string(),
            match_value: body.to_string(),
            position: span_to_pos(self.source, start, end),
            file_type: self.file_type,
            file_path: self.file_path.map(str::to_string),
            severity: Severity::Low,
            provenance: Provenance::Ast,
            metadata: Some(gql_metadata(body)),
        });
    }
}

impl<'a, 's> Visit<'a> for GqlVisitor<'s> {
    fn visit_string_literal(&mut self, lit: &StringLiteral<'a>) {
        if lit.value.len() < 16 || lit.value.len() > 16 * 1024 {
            return;
        }
        self.maybe_emit(lit.value.as_str(), lit.span.start, lit.span.end);
    }

    fn visit_template_literal(&mut self, lit: &TemplateLiteral<'a>) {
        let joined: String = lit
            .quasis
            .iter()
            .filter_map(|q| q.value.cooked.as_ref().map(|s| s.as_str()))
            .collect::<Vec<_>>()
            .join("");
        if joined.len() >= 16 {
            self.maybe_emit(&joined, lit.span.start, lit.span.end);
        }
        walk::walk_template_literal(self, lit);
    }

    fn visit_tagged_template_expression(&mut self, expr: &TaggedTemplateExpression<'a>) {
        // `gql\`...\`` with a `gql` tag — be more eager (skip keyword check).
        let tag_name = match &expr.tag {
            Expression::Identifier(id) => Some(id.name.as_str()),
            _ => None,
        };

        if matches!(tag_name, Some("gql" | "graphql")) {
            let joined: String = expr
                .quasi
                .quasis
                .iter()
                .filter_map(|q| q.value.cooked.as_ref().map(|s| s.as_str()))
                .collect::<Vec<_>>()
                .join("");
            let meta = gql_metadata(&joined);
            self.out.push(AnalyzerMatch {
                match_kind: "graphql".to_string(),
                match_value: joined,
                position: span_to_pos(self.source, expr.span.start, expr.span.end),
                file_type: self.file_type,
                file_path: self.file_path.map(str::to_string),
                severity: Severity::Low,
                provenance: Provenance::Ast,
                metadata: Some(meta),
            });
        }

        walk::walk_tagged_template_expression(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_named_query() {
        let src = r#"const Q = "query GetUser($id: ID!) { user(id: $id) { id name } }";"#;
        let ms = analyze(src, None, FileType::Js);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].match_kind, "graphql");
    }

    #[test]
    fn finds_mutation() {
        let src = r#"const M = "mutation { setName(name: \"x\") { ok } }";"#;
        let ms = analyze(src, None, FileType::Js);
        assert_eq!(ms.len(), 1);
    }

    #[test]
    fn finds_gql_tagged_template() {
        let src = r#"
            import gql from 'graphql-tag';
            const Q = gql`query { me { id } }`;
        "#;
        let ms = analyze(src, None, FileType::Js);
        assert!(!ms.is_empty());
    }

    #[test]
    fn ignores_non_gql_templates() {
        let src = r#"const x = `not a query`;"#;
        let ms = analyze(src, None, FileType::Js);
        assert!(ms.is_empty());
    }
}
