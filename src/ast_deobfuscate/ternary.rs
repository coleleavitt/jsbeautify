use oxc_ast::ast::Expression;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct TernarySimplifier {
    changed: bool,
}

impl TernarySimplifier {
    #[must_use]
    pub const fn new() -> Self {
        Self { changed: false }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn get_constant_boolean(&self, expr: &Expression<'_>) -> Option<bool> {
        match expr {
            Expression::BooleanLiteral(b) => Some(b.value),
            Expression::NumericLiteral(n) => Some(n.value != 0.0),
            Expression::StringLiteral(s) => Some(!s.value.is_empty()),
            Expression::NullLiteral(_) => Some(false),
            _ => None,
        }
    }
}

impl Default for TernarySimplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for TernarySimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::ConditionalExpression(cond) = expr
            && let Some(condition_value) = self.get_constant_boolean(&cond.test)
        {
            eprintln!("[AST] Simplifying ternary with constant condition: {condition_value}");
            self.changed = true;

            let branch = if condition_value {
                std::mem::replace(
                    &mut cond.consequent,
                    Expression::new_null_literal(oxc_span::SPAN, &ctx.ast),
                )
            } else {
                std::mem::replace(
                    &mut cond.alternate,
                    Expression::new_null_literal(oxc_span::SPAN, &ctx.ast),
                )
            };

            *expr = branch;
        }
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

    fn run_ternary_simplifier(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut simplifier = TernarySimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_simplify_true_ternary() {
        let output = run_ternary_simplifier("var x = true ? 1 : 2;");
        eprintln!("Output: {output}");
        assert!(
            output.contains("= 1") || output.contains("=1"),
            "Should keep true branch, got: {output}"
        );
        assert!(!output.contains('?'), "Should remove ternary, got: {output}");
    }

    #[test]
    fn test_simplify_false_ternary() {
        let output = run_ternary_simplifier("var x = false ? 1 : 2;");
        eprintln!("Output: {output}");
        assert!(
            output.contains("= 2") || output.contains("=2"),
            "Should keep false branch, got: {output}"
        );
        assert!(!output.contains('?'), "Should remove ternary, got: {output}");
    }

    #[test]
    fn test_simplify_truthy_number() {
        let output = run_ternary_simplifier("var x = 1 ? 'yes' : 'no';");
        eprintln!("Output: {output}");
        assert!(
            output.contains("yes"),
            "Should keep true branch for truthy number, got: {output}"
        );
    }

    #[test]
    fn test_simplify_falsy_zero() {
        let output = run_ternary_simplifier("var x = 0 ? 'yes' : 'no';");
        eprintln!("Output: {output}");
        assert!(
            output.contains("no"),
            "Should keep false branch for falsy zero, got: {output}"
        );
    }

    #[test]
    fn test_preserve_non_constant() {
        let output = run_ternary_simplifier("var x = condition ? 1 : 2;");
        eprintln!("Output: {output}");
        assert!(
            output.contains('?'),
            "Should preserve non-constant ternary, got: {output}"
        );
    }
}
