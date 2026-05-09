//! NPM package name extraction.
//!
//! Ports `crates/core/analyzers/src/js_ast_npm_package_analyzer.rs` and
//! `js_ast_package_json_analyzer.rs`. Looks at:
//! - `import x from "pkg"`
//! - `require("pkg")`
//! - `package.json`-style object literals with `dependencies` keys.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::helpers;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

const PACKAGE_JSON_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// Extract NPM package usages.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = NpmVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

struct NpmVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'s> NpmVisitor<'s> {
    fn maybe_emit_package(&mut self, name: &str, start: u32, end: u32) {
        if name.is_empty() || name.len() > 256 {
            return;
        }
        if name.starts_with('.') || name.starts_with('/') {
            return; // relative or absolute path import
        }
        // Strip subpath first: "lodash/get" → "lodash", "@scope/pkg/x" → "@scope/pkg"
        let pkg = if name.starts_with('@') {
            // @scope/pkg[/...]
            let mut parts = name.splitn(3, '/');
            match (parts.next(), parts.next()) {
                (Some(scope), Some(pkg_name)) => format!("{scope}/{pkg_name}"),
                _ => name.to_string(),
            }
        } else {
            name.split('/').next().unwrap_or(name).to_string()
        };
        // Validate the stripped package name
        if !helpers::PACKAGE_NAME_REGEX.is_match(&pkg) {
            return;
        }
        self.out.push(AnalyzerMatch {
            match_kind: "npm_package".to_string(),
            match_value: pkg,
            position: span_to_pos(self.source, start, end),
            file_type: self.file_type,
            file_path: self.file_path.map(str::to_string),
            severity: Severity::Info,
            provenance: Provenance::Ast,
            metadata: None,
        });
    }
}

impl<'a, 's> Visit<'a> for NpmVisitor<'s> {
    fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
        let src = decl.source.value.as_str();
        self.maybe_emit_package(src, decl.source.span.start, decl.source.span.end);
        walk::walk_import_declaration(self, decl);
    }

    fn visit_export_named_declaration(&mut self, decl: &ExportNamedDeclaration<'a>) {
        if let Some(s) = &decl.source {
            self.maybe_emit_package(s.value.as_str(), s.span.start, s.span.end);
        }
        walk::walk_export_named_declaration(self, decl);
    }

    fn visit_export_all_declaration(&mut self, decl: &ExportAllDeclaration<'a>) {
        self.maybe_emit_package(decl.source.value.as_str(), decl.source.span.start, decl.source.span.end);
        walk::walk_export_all_declaration(self, decl);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // require("pkg")  or  require.resolve("pkg")
        let is_require = match &call.callee {
            Expression::Identifier(id) => id.name == "require",
            Expression::StaticMemberExpression(sm) => {
                matches!(&sm.object, Expression::Identifier(id) if id.name == "require")
                    && sm.property.name == "resolve"
            }
            _ => false,
        };
        if is_require {
            if let Some(Argument::StringLiteral(s)) = call.arguments.first() {
                self.maybe_emit_package(s.value.as_str(), s.span.start, s.span.end);
            }
        }

        walk::walk_call_expression(self, call);
    }

    // Dynamic import(): import("pkg")
    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        if let Expression::StringLiteral(s) = &it.source {
            self.maybe_emit_package(s.value.as_str(), s.span.start, s.span.end);
        }
        walk::walk_import_expression(self, it);
    }

    fn visit_object_expression(&mut self, obj: &ObjectExpression<'a>) {
        // package.json-style: { "dependencies": { "react": "^18", ... } }
        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop {
                let key_name = match &p.key {
                    PropertyKey::StringLiteral(s) => Some(s.value.as_str()),
                    PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                    _ => None,
                };
                if let Some(key) = key_name {
                    if PACKAGE_JSON_KEYS.contains(&key) {
                        // The value should be an ObjectExpression of pkg → version
                        if let Expression::ObjectExpression(deps) = &p.value {
                            for dep in &deps.properties {
                                if let ObjectPropertyKind::ObjectProperty(dp) = dep {
                                    let pname = match &dp.key {
                                        PropertyKey::StringLiteral(s) => {
                                            Some((s.value.as_str(), s.span.start, s.span.end))
                                        }
                                        PropertyKey::StaticIdentifier(id) => {
                                            Some((id.name.as_str(), id.span.start, id.span.end))
                                        }
                                        _ => None,
                                    };
                                    if let Some((n, s, e)) = pname {
                                        self.maybe_emit_package(n, s, e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        walk::walk_object_expression(self, obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .map(|m| m.match_value)
            .collect()
    }

    #[test]
    fn finds_static_imports() {
        let n = names(r#"import React from "react"; import _ from "lodash";"#);
        assert!(n.contains(&"react".to_string()));
        assert!(n.contains(&"lodash".to_string()));
    }

    #[test]
    fn handles_subpaths() {
        let n = names(r#"import x from "lodash/get";"#);
        assert!(n.contains(&"lodash".to_string()));
    }

    #[test]
    fn handles_scoped() {
        let n = names(r#"import x from "@babel/core/lib/parse";"#);
        assert!(n.contains(&"@babel/core".to_string()));
    }

    #[test]
    fn finds_require_calls() {
        let n = names(r#"const x = require("express");"#);
        assert!(n.contains(&"express".to_string()));
    }

    #[test]
    fn finds_dynamic_imports() {
        let n = names(r#"const m = await import("axios");"#);
        assert!(n.contains(&"axios".to_string()));
    }

    #[test]
    fn skips_relative_imports() {
        let n = names(r#"import x from "./foo"; import y from "../bar";"#);
        assert!(n.is_empty());
    }

    #[test]
    fn finds_package_json_deps() {
        let n = names(r#"const pkg = { dependencies: { "react": "18", "axios": "1.0" } };"#);
        assert!(n.contains(&"react".to_string()));
        assert!(n.contains(&"axios".to_string()));
    }
}
