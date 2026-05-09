//! Path/URL/hostname extraction from JS string literals (and HTML comments).
//!
//! Ports `crates/core/analyzers/src/js_ast_path_analyzer.rs`,
//! `js_ast_hostname_analyzer.rs`, and the comment variants from jxscout.
//!
//! Strategy: parse with oxc, walk the AST, look at every `StringLiteral` and
//! `TemplateLiteral`, and apply the noise filter from [`super::helpers`].

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::helpers;
use super::match_kind::{AnalyzerMatch, FileType};
use super::regex_bank::Severity;

/// Extract paths, hostnames, and URLs from `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = PathHostVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);

    // Comment scan
    for c in &parse.program.comments {
        let text = &source[c.span.start as usize..c.span.end as usize];
        visitor.harvest_comment(text, c.span.start);
    }

    visitor.out
}

struct PathHostVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'s> PathHostVisitor<'s> {
    fn harvest_string(&mut self, value: &str, span_start: u32, span_end: u32) {
        if value.is_empty() || value.len() > 2048 {
            return;
        }
        if helpers::is_path_noise(value) {
            return;
        }

        // URL match (full URL with scheme)
        for m in helpers::URL_PATTERN.find_iter(value) {
            let s = m.as_str();
            // Reject URLs to ignored hosts
            if let Ok(parsed) = url::Url::parse(s) {
                if let Some(host) = parsed.host_str() {
                    if helpers::is_hostname_noise(host) {
                        continue;
                    }
                }
            }
            self.out.push(AnalyzerMatch {
                match_kind: "url".into(),
                match_value: s.to_string(),
                position: span_to_pos(self.source, span_start, span_end),
                file_type: self.file_type,
                file_path: self.file_path.map(str::to_string),
                severity: Severity::Info,
                metadata: None,
            });
        }

        // Hostname match (bare host without scheme)
        if let Some(caps) = helpers::HOSTNAME_PATTERN.captures(value) {
            if let Some(g) = caps.get(1) {
                let host = g.as_str();
                if !helpers::is_hostname_noise(host) && host.contains('.') && !host.starts_with('.') {
                    // Skip if it's the same content the URL_PATTERN already captured
                    let already_url = self
                        .out
                        .iter()
                        .any(|m| m.match_kind == "url" && m.match_value.contains(host));
                    if !already_url {
                        self.out.push(AnalyzerMatch {
                            match_kind: "hostname".into(),
                            match_value: host.to_string(),
                            position: span_to_pos(self.source, span_start, span_end),
                            file_type: self.file_type,
                            file_path: self.file_path.map(str::to_string),
                            severity: Severity::Info,
                            metadata: None,
                        });
                    }
                }
            }
        }

        // Path match (must start with / or alphanum and contain /)
        for m in helpers::PATH_PATTERN.find_iter(value) {
            let path = m.as_str();
            if path.len() < 2 || helpers::is_path_noise(path) {
                continue;
            }
            // Don't double-report URLs / hostnames as paths.
            if path.contains("://") || (!path.contains('/') && !path.contains(':')) {
                continue;
            }
            // Skip noise prefixes
            if path.starts_with('.') || path.starts_with('+') || path.starts_with('#') || path.starts_with("${") {
                continue;
            }
            self.out.push(AnalyzerMatch {
                match_kind: "path".into(),
                match_value: path.to_string(),
                position: span_to_pos(self.source, span_start, span_end),
                file_type: self.file_type,
                file_path: self.file_path.map(str::to_string),
                severity: Severity::Info,
                metadata: None,
            });
        }
    }

    fn harvest_comment(&mut self, text: &str, base_start: u32) {
        // For comments we use comment_path / comment_hostname / comment_url kinds.
        if let Some(caps) = helpers::HOSTNAME_PATTERN.captures(text) {
            if let Some(g) = caps.get(1) {
                let host = g.as_str();
                if !helpers::is_hostname_noise(host) && host.contains('.') {
                    self.out.push(AnalyzerMatch {
                        match_kind: "comment_hostname".into(),
                        match_value: host.to_string(),
                        position: span_to_pos(self.source, base_start, base_start + text.len() as u32),
                        file_type: self.file_type,
                        file_path: self.file_path.map(str::to_string),
                        severity: Severity::Info,
                        metadata: None,
                    });
                }
            }
        }
        for m in helpers::PATH_PATTERN.find_iter(text) {
            let p = m.as_str();
            if p.len() >= 3 && p.contains('/') && !helpers::is_path_noise(p) && !p.contains("://") {
                self.out.push(AnalyzerMatch {
                    match_kind: "comment_path".into(),
                    match_value: p.to_string(),
                    position: span_to_pos(self.source, base_start, base_start + text.len() as u32),
                    file_type: self.file_type,
                    file_path: self.file_path.map(str::to_string),
                    severity: Severity::Info,
                    metadata: None,
                });
            }
        }
    }
}

impl<'a, 's> Visit<'a> for PathHostVisitor<'s> {
    fn visit_string_literal(&mut self, lit: &StringLiteral<'a>) {
        self.harvest_string(lit.value.as_str(), lit.span.start, lit.span.end);
    }

    fn visit_template_literal(&mut self, lit: &TemplateLiteral<'a>) {
        // Only the static parts are reliable.
        for q in &lit.quasis {
            if let Some(cooked) = q.value.cooked.as_ref() {
                self.harvest_string(cooked.as_str(), q.span.start, q.span.end);
            }
        }
        walk::walk_template_literal(self, lit);
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
    fn finds_basic_url() {
        let src = r#"const u = "https://api.target.com/v2/users";"#;
        let k = kinds(src);
        assert!(k.iter().any(|s| s == "url"), "wanted url; got {k:?}");
    }

    #[test]
    fn extracts_path_alone() {
        let src = r#"fetch("/api/v2/admin/secret");"#;
        let k = kinds(src);
        assert!(k.iter().any(|s| s == "path"), "wanted path; got {k:?}");
    }

    #[test]
    fn ignores_node_module_imports() {
        let src = r#"import x from "./node_modules/foo";"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.iter().all(|m| m.match_value != "./node_modules/foo"));
    }

    #[test]
    fn ignores_timezone_strings() {
        let src = r#"const tz = "America/New_York";"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.iter().all(|m| m.match_value != "America/New_York"));
    }

    #[test]
    fn ignores_w3_namespace() {
        let src = r#"const ns = "http://www.w3.org/2000/svg";"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.iter().all(|m| !m.match_value.contains("www.w3.org")));
    }

    #[test]
    fn template_literal_static_parts() {
        let src = r#"const x = `hello /api/v2/${user}/items`;"#;
        let k = kinds(src);
        assert!(k.iter().any(|s| s == "path"));
    }
}
