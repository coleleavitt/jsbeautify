use oxc_ast::ast::{BooleanLiteral, Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct BooleanLiteralConverter {
    changed: bool,
    double_negation_count: usize,
}

impl BooleanLiteralConverter {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            changed: false,
            double_negation_count: 0,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    #[must_use]
    pub const fn double_negation_count(&self) -> usize {
        self.double_negation_count
    }
}

impl Default for BooleanLiteralConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for BooleanLiteralConverter {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::UnaryExpression(unary) = expr else {
            return;
        };
        if unary.operator != UnaryOperator::LogicalNot {
            return;
        }

        // Check for !!x pattern BEFORE checking for !<literal>
        if let Expression::UnaryExpression(inner) = &unary.argument {
            if inner.operator == UnaryOperator::LogicalNot {
                self.double_negation_count += 1;
                // Do NOT rewrite !!x — it's idiomatic JavaScript
                return;
            }
        }

        let Some(truthy) = js_truthiness(&unary.argument) else {
            return;
        };
        let result = !truthy;
        eprintln!("[AST] Converting !<literal> -> {result}");
        self.changed = true;
        *expr = Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: result,
        }));
    }
}

fn js_truthiness(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::NumericLiteral(n) => Some(n.value != 0.0 && !n.value.is_nan()),
        Expression::StringLiteral(s) => Some(!s.value.is_empty()),
        Expression::BooleanLiteral(b) => Some(b.value),
        Expression::NullLiteral(_) => Some(false),
        Expression::Identifier(id) => match id.name.as_str() {
            "undefined" | "NaN" => Some(false),
            "Infinity" => Some(true),
            _ => None,
        },
        Expression::ArrayExpression(_) | Expression::ObjectExpression(_) => Some(true),
        Expression::ParenthesizedExpression(p) => js_truthiness(&p.expression),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_boolean_literals(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut converter = BooleanLiteralConverter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_not_zero_to_true() {
        let output = run_boolean_literals("var x = !0;");
        eprintln!("Output: {output}");
        assert!(output.contains("true"), "Expected true, got: {output}");
    }

    #[test]
    fn test_not_one_to_false() {
        let output = run_boolean_literals("var x = !1;");
        eprintln!("Output: {output}");
        assert!(output.contains("false"), "Expected false, got: {output}");
    }

    #[test]
    fn test_not_positive_number_to_false() {
        let output = run_boolean_literals("var x = !5;");
        assert!(output.contains("false"), "!5 -> false, got: {output}");
    }

    #[test]
    fn test_not_string_literal() {
        assert!(run_boolean_literals("var x = !\"hi\";").contains("false"));
        assert!(run_boolean_literals("var x = !\"\";").contains("true"));
    }

    #[test]
    fn test_not_array_object() {
        assert!(run_boolean_literals("var x = ![];").contains("false"));
        assert!(run_boolean_literals("var x = !{};").contains("false"));
    }

    #[test]
    fn test_not_undefined_nan_null() {
        assert!(run_boolean_literals("var x = !undefined;").contains("true"));
        assert!(run_boolean_literals("var x = !NaN;").contains("true"));
        assert!(run_boolean_literals("var x = !null;").contains("true"));
    }

    #[test]
    fn test_preserve_not_variable() {
        let output = run_boolean_literals("var x = !foo;");
        eprintln!("Output: {output}");
        assert!(output.contains("!foo"), "Should preserve !foo, got: {output}");
    }

    #[test]
    fn test_counts_double_negation() {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let code = "var x = !!y; var z = !!w;";
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut converter = BooleanLiteralConverter::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut converter, &mut program, &mut ctx);

        let output = Codegen::new().build(&program).code;
        eprintln!("Output: {output}");
        assert!(output.contains("!!y"), "Should preserve !!y, got: {output}");
        assert!(output.contains("!!w"), "Should preserve !!w, got: {output}");
        assert_eq!(converter.double_negation_count(), 2, "Expected 2 double-negations");
    }
}
