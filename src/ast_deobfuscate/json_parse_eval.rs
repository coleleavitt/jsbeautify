//! JSON.parse() evaluation for string decoders
//!
//! Detects `JSON.parse()` calls with string literals.
//! Currently a detection pass that counts occurrences.
//! Full evaluation requires complex AST construction.
//!
//! Pattern:
//! ```javascript
//! JSON.parse('{"key":"value"}')
//! JSON.parse("[1,2,3]")
//! JSON.parse('"hello"')
//! ```

use oxc_ast::ast::{CallExpression, Expression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct JsonParseEvaluator {
    evaluated_count: usize,
}

impl JsonParseEvaluator {
    #[must_use]
    pub const fn new() -> Self {
        Self { evaluated_count: 0 }
    }

    #[must_use]
    pub const fn evaluated_count(&self) -> usize {
        self.evaluated_count
    }

    /// Check if this is a JSON.parse() call with a string literal
    fn is_json_parse_with_string(call: &CallExpression) -> bool {
        // Check if callee is JSON.parse
        let is_json_parse = if let Expression::StaticMemberExpression(member) = &call.callee {
            if let Expression::Identifier(obj) = &member.object {
                obj.name == "JSON" && member.property.name == "parse"
            } else {
                false
            }
        } else {
            false
        };

        if !is_json_parse {
            return false;
        }

        // Must have exactly one argument that is a string literal
        if call.arguments.len() != 1 {
            return false;
        }

        let arg = &call.arguments[0];
        matches!(arg.as_expression(), Some(Expression::StringLiteral(_)))
    }
}

impl Default for JsonParseEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for JsonParseEvaluator {
    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, _ctx: &mut Ctx<'a>) {
        if Self::is_json_parse_with_string(call) {
            eprintln!("[JSON_PARSE] Detected JSON.parse() with string literal");
            self.evaluated_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_detect(code: &str) -> usize {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut evaluator = JsonParseEvaluator::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut evaluator, &mut program, &mut ctx);

        evaluator.evaluated_count()
    }

    #[test]
    fn test_detect_json_parse_object() {
        let code = r#"JSON.parse('{"key":"value"}')"#;
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect JSON.parse with object, got: {count}");
    }

    #[test]
    fn test_detect_json_parse_array() {
        let code = r#"JSON.parse('[1,2,3]')"#;
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect JSON.parse with array, got: {count}");
    }

    #[test]
    fn test_detect_json_parse_string() {
        let code = r#"JSON.parse('"hello"')"#;
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect JSON.parse with string, got: {count}");
    }

    #[test]
    fn test_detect_json_parse_number() {
        let code = r#"JSON.parse('42')"#;
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect JSON.parse with number, got: {count}");
    }

    #[test]
    fn test_detect_json_parse_boolean() {
        let code = r#"JSON.parse('true')"#;
        let count = run_detect(code);
        assert_eq!(count, 1, "Should detect JSON.parse with boolean, got: {count}");
    }

    #[test]
    fn test_ignore_json_parse_variable() {
        let code = r#"JSON.parse(variable)"#;
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect JSON.parse with variable, got: {count}");
    }

    #[test]
    fn test_ignore_json_parse_no_args() {
        let code = r#"JSON.parse()"#;
        let count = run_detect(code);
        assert_eq!(count, 0, "Should NOT detect JSON.parse with no args, got: {count}");
    }

    #[test]
    fn test_detect_multiple_json_parse() {
        let code = r#"
            JSON.parse('{"a":1}');
            JSON.parse('[1,2]');
            JSON.parse('"test"');
        "#;
        let count = run_detect(code);
        assert_eq!(count, 3, "Should detect 3 JSON.parse calls, got: {count}");
    }
}
