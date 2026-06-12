//! `package.json` block detection.
//!
//! Ports `crates/core/analyzers/src/js_ast_package_json_analyzer.rs` from
//! jxscout-pro-v2 (REPORT_03 row 10).
//!
//! Emits `package_json` matches whenever an object literal contains at least
//! one of the four dependency-block keys:
//!   - `dependencies`
//!   - `devDependencies`
//!   - `peerDependencies`
//!   - `optionalDependencies`
//!
//! Two emission strategies:
//!  1. One match per dependency *block* (kind `package_json`), value =
//!     the block key, metadata = `{ "packages": [...], "block": "..." }`.
//!  2. One match per dependency *entry* (kind `package_json_dep`), value =
//!     `"<name>@<version>"`, metadata = `{ "name", "version", "block" }`.
//!
//! Both are emitted so downstream tooling can consume whichever it prefers.
//! This catches inlined `package.json` content in bundled JS (a common
//! exposure pattern where `import "./package.json"` ships the whole manifest
//! into the browser bundle).

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

const DEP_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

const KIND_BLOCK: &str = "package_json";
const KIND_ENTRY: &str = "package_json_dep";

/// Detect package.json-style `dependencies` blocks in `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = PkgVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);

    // Also try to parse the input as raw JSON in case caller passed a literal
    // package.json file. We don't fail if it's not JSON.
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(source) {
        if let serde_json::Value::Object(map) = json {
            for &key in DEP_KEYS {
                if let Some(serde_json::Value::Object(deps)) = map.get(key) {
                    let names: Vec<String> = deps.keys().cloned().collect();
                    if names.is_empty() {
                        continue;
                    }
                    visitor.out.push(AnalyzerMatch {
                        match_kind: KIND_BLOCK.to_string(),
                        match_value: key.to_string(),
                        position: span_to_pos(source, 0, source.len() as u32),
                        file_type,
                        file_path: file_path.map(str::to_string),
                        severity: Severity::Medium,
                        provenance: Provenance::Ast,
                        metadata: Some(serde_json::json!({
                            "block": key,
                            "packages": names,
                        })),
                    });
                    for (name, ver) in deps {
                        let version = match ver {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        visitor.out.push(AnalyzerMatch {
                            match_kind: KIND_ENTRY.to_string(),
                            match_value: format!("{name}@{version}"),
                            position: span_to_pos(source, 0, source.len() as u32),
                            file_type,
                            file_path: file_path.map(str::to_string),
                            severity: Severity::Info,
                            provenance: Provenance::Ast,
                            metadata: Some(serde_json::json!({
                                "name": name,
                                "version": version,
                                "block": key,
                            })),
                        });
                    }
                }
            }
        }
    }

    visitor.out
}

struct PkgVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'s> PkgVisitor<'s> {
    fn handle_object(&mut self, obj: &ObjectExpression<'_>) {
        for prop in &obj.properties {
            let ObjectPropertyKind::ObjectProperty(op) = prop else {
                continue;
            };
            let key_name = match &op.key {
                PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                PropertyKey::StringLiteral(s) => s.value.as_str(),
                _ => continue,
            };
            if !DEP_KEYS.contains(&key_name) {
                continue;
            }
            // The value should itself be an object literal of name -> version.
            let Expression::ObjectExpression(inner) = &op.value else {
                continue;
            };

            let mut names: Vec<String> = Vec::new();
            let mut entries: Vec<(String, String)> = Vec::new();
            for inner_prop in &inner.properties {
                let ObjectPropertyKind::ObjectProperty(ip) = inner_prop else {
                    continue;
                };
                let pkg_name = match &ip.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                    PropertyKey::StringLiteral(s) => s.value.as_str(),
                    _ => continue,
                };
                let version = match &ip.value {
                    Expression::StringLiteral(s) => s.value.to_string(),
                    other => other.span().source_text(self.source).to_string(),
                };
                names.push(pkg_name.to_string());
                entries.push((pkg_name.to_string(), version));
            }

            if names.is_empty() {
                continue;
            }

            self.out.push(AnalyzerMatch {
                match_kind: KIND_BLOCK.to_string(),
                match_value: key_name.to_string(),
                position: span_to_pos(self.source, inner.span.start, inner.span.end),
                file_type: self.file_type,
                file_path: self.file_path.map(str::to_string),
                severity: Severity::Medium,
                provenance: Provenance::Ast,
                metadata: Some(serde_json::json!({
                    "block": key_name,
                    "packages": names,
                })),
            });

            for (name, version) in entries {
                self.out.push(AnalyzerMatch {
                    match_kind: KIND_ENTRY.to_string(),
                    match_value: format!("{name}@{version}"),
                    position: span_to_pos(self.source, inner.span.start, inner.span.end),
                    file_type: self.file_type,
                    file_path: self.file_path.map(str::to_string),
                    severity: Severity::Info,
                    provenance: Provenance::Ast,
                    metadata: Some(serde_json::json!({
                        "name": name,
                        "version": version,
                        "block": key_name,
                    })),
                });
            }
        }
    }
}

impl<'a, 's> Visit<'a> for PkgVisitor<'s> {
    fn visit_object_expression(&mut self, obj: &ObjectExpression<'a>) {
        self.handle_object(obj);
        walk::walk_object_expression(self, obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_inline_package_json_in_js() {
        let src = r#"
            const manifest = {
                name: "my-app",
                dependencies: {
                    "react": "^18.2.0",
                    "lodash": "4.17.21"
                },
                devDependencies: {
                    "typescript": "5.0.4"
                }
            };
        "#;
        let m = analyze(src, Some("test.js"), FileType::Js);
        let kinds: Vec<&str> = m.iter().map(|x| x.match_kind.as_str()).collect();
        assert!(kinds.contains(&"package_json"));
        assert!(kinds.contains(&"package_json_dep"));

        let blocks: Vec<_> = m.iter().filter(|x| x.match_kind == "package_json").collect();
        assert_eq!(blocks.len(), 2, "should find two dep blocks");

        let entries: Vec<_> = m
            .iter()
            .filter(|x| x.match_kind == "package_json_dep")
            .map(|x| x.match_value.clone())
            .collect();
        assert!(entries.iter().any(|v| v.starts_with("react@")));
        assert!(entries.iter().any(|v| v.starts_with("typescript@")));
    }

    #[test]
    fn finds_package_json_as_raw_json() {
        let src = r#"{
            "name": "x",
            "version": "0.1.0",
            "dependencies": { "axios": "^1.4.0" },
            "peerDependencies": { "react": "^18" }
        }"#;
        let m = analyze(src, Some("package.json"), FileType::Js);
        let blocks: Vec<&str> = m
            .iter()
            .filter(|x| x.match_kind == "package_json")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(blocks.contains(&"dependencies"));
        assert!(blocks.contains(&"peerDependencies"));
    }

    #[test]
    fn empty_object_no_match() {
        let m = analyze("const x = {};", None, FileType::Js);
        assert!(m.is_empty());
    }

    #[test]
    fn ignores_non_dep_keys() {
        let src = r#"const x = { config: { foo: "bar" } };"#;
        let m = analyze(src, None, FileType::Js);
        assert!(m.is_empty());
    }
}
