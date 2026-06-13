//! Client-side prototype-pollution candidate detection (CWE-1321).
//!
//! Prototype pollution lets an attacker inject properties onto `Object.prototype`
//! via unsafe property writes with attacker-controlled keys. The two structural
//! signatures, per Silent Spring / GHunter, are:
//!
//!  1. **Dynamic recursive merge / deep-assign**: `dst[k] = src[k]` (or
//!     `dst[a][b] = v`) inside a loop over attacker keys — the classic
//!     `merge`/`extend`/`deepClone` sink.
//!  2. **Computed property write reaching a dangerous key**: `obj[k1][k2] = v`
//!     where `k1`/`k2` can be `__proto__` / `constructor` / `prototype`.
//!
//! We flag the *bracket-write with a non-literal (computed) key* — `a[k] = v` and
//! `a[k1][k2] = v` — as `proto_pollution` candidates, and raise severity when a
//! literal dangerous key (`__proto__`, `constructor`, `prototype`) appears. This
//! is an over-approximation (a lead, not a proof); a literal safe key like
//! `a["id"] = v` is excluded.

#![allow(clippy::doc_markdown)]

use oxc_allocator::Allocator;
use oxc_ast::ast::{AssignmentExpression, AssignmentTarget, Expression, MemberExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

/// Dangerous keys that pollute the prototype chain.
const DANGEROUS_KEYS: &[&str] = &["__proto__", "constructor", "prototype"];

/// Scan `source` for prototype-pollution candidate writes.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();
    let mut visitor = ProtoVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

struct ProtoVisitor<'s> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl ProtoVisitor<'_> {
    fn push(&mut self, span: Span, severity: Severity, reason: &str) {
        let snippet = span.source_text(self.source).chars().take(160).collect::<String>();
        self.out.push(
            AnalyzerMatch::new(
                "proto_pollution",
                snippet,
                span_to_pos(self.source, span.start, span.end),
                self.file_type,
                self.file_path.map(String::from),
            )
            .with_provenance(Provenance::Ast)
            .with_severity(severity)
            .with_metadata(serde_json::json!({ "reason": reason })),
        );
    }
}

/// The literal property key of a computed member, if it is a string literal.
fn literal_computed_key<'a>(member: &'a MemberExpression<'a>) -> Option<&'a str> {
    match member {
        MemberExpression::ComputedMemberExpression(c) => match &c.expression {
            Expression::StringLiteral(s) => Some(s.value.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a computed member uses a *non-literal* (variable) key — the shape
/// that lets an attacker control the property name.
fn has_dynamic_key(member: &MemberExpression<'_>) -> bool {
    matches!(member, MemberExpression::ComputedMemberExpression(c) if !matches!(c.expression, Expression::StringLiteral(_) | Expression::NumericLiteral(_)))
}

impl<'a> Visit<'a> for ProtoVisitor<'_> {
    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        if let AssignmentTarget::ComputedMemberExpression(c) = &assign.left {
            // a[k] = v  — is `k` a dangerous literal, or a dynamic key?
            if let Expression::StringLiteral(s) = &c.expression {
                if DANGEROUS_KEYS.contains(&s.value.as_str()) {
                    self.push(
                        assign.span,
                        Severity::High,
                        "write to a prototype-polluting literal key",
                    );
                }
            } else if !matches!(c.expression, Expression::NumericLiteral(_)) {
                // Dynamic key. Stronger signal if the object is itself a computed
                // member (a[k1][k2] = v), the canonical deep-merge sink.
                let nested = matches!(&c.object, Expression::ComputedMemberExpression(_));
                let inner_dangerous = if let Expression::StaticMemberExpression(_)
                | Expression::ComputedMemberExpression(_) = &c.object
                {
                    c.object
                        .as_member_expression()
                        .and_then(literal_computed_key)
                        .is_some_and(|k| DANGEROUS_KEYS.contains(&k))
                } else {
                    false
                };
                let inner_dynamic = c.object.as_member_expression().is_some_and(has_dynamic_key);
                if inner_dangerous {
                    self.push(
                        assign.span,
                        Severity::High,
                        "nested write under a dangerous key with dynamic key",
                    );
                } else if nested || inner_dynamic {
                    self.push(
                        assign.span,
                        Severity::Medium,
                        "nested dynamic-key property write (deep-merge sink shape)",
                    );
                } else {
                    self.push(assign.span, Severity::Low, "dynamic-key property write");
                }
            }
        }
        walk::walk_assignment_expression(self, assign);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_and_sev(src: &str) -> Vec<(String, Severity)> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .map(|m| (m.match_kind, m.severity))
            .collect()
    }

    #[test]
    fn flags_literal_proto_write() {
        let v = kinds_and_sev(r#"obj["__proto__"] = payload;"#);
        assert!(
            v.iter().any(|(k, s)| k == "proto_pollution" && *s == Severity::High),
            "{v:?}"
        );
    }

    #[test]
    fn flags_nested_dynamic_merge_shape() {
        // dst[a][b] = src[a][b] — the deep-merge sink.
        let src = "function merge(dst, k1, k2, v){ dst[k1][k2] = v; }";
        let v = kinds_and_sev(src);
        assert!(
            v.iter().any(|(k, s)| k == "proto_pollution" && *s == Severity::Medium),
            "{v:?}"
        );
    }

    #[test]
    fn flags_dynamic_under_dangerous_key() {
        let src = r#"obj["__proto__"][k] = v;"#;
        let v = kinds_and_sev(src);
        assert!(v.iter().any(|(_, s)| *s == Severity::High), "{v:?}");
    }

    #[test]
    fn ignores_safe_literal_and_numeric_keys() {
        assert!(analyze(r#"obj["id"] = 1;"#, None, FileType::Js).is_empty());
        assert!(analyze("arr[0] = 1;", None, FileType::Js).is_empty());
        assert!(analyze("obj.normal = 1;", None, FileType::Js).is_empty());
    }
}
