//! Comma-return simplifier pass
//!
//! Simplifies sequence expressions where the last element is a redundant
//! identifier that matches the LHS of the preceding assignment:
//!
//! ```js
//! (Ot.pop(), jC9 = Jc9, jC9)   // before
//! (Ot.pop(), jC9 = Jc9)         // after — assignment already evaluates to Jc9
//! ```
//!
//! Safe because `X = Y` evaluates to `Y` in JavaScript, so the trailing
//! identifier adds no value.

use oxc_ast::ast::{AssignmentTarget, Expression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct CommaReturnSimplifier {
    simplified: usize,
}

impl CommaReturnSimplifier {
    #[must_use]
    pub const fn new() -> Self {
        Self { simplified: 0 }
    }

    #[must_use]
    pub const fn simplified(&self) -> usize {
        self.simplified
    }
}

impl Default for CommaReturnSimplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CommaReturnSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, _ctx: &mut Ctx<'a>) {
        let Expression::SequenceExpression(seq) = expr else {
            return;
        };
        if seq.expressions.len() < 2 {
            return;
        }

        let len = seq.expressions.len();
        let last = &seq.expressions[len - 1];
        let second_last = &seq.expressions[len - 2];

        if let Expression::Identifier(last_id) = last
            && let Expression::AssignmentExpression(assign) = second_last
            && let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left
            && target.name.as_str() == last_id.name.as_str()
        {
            seq.expressions.pop();
            self.simplified += 1;
            if self.simplified <= 10 {
                eprintln!("[AST/comma-return] simplified redundant trailing identifier");
            }
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

    fn run_simplify(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut simplifier = CommaReturnSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

        (Codegen::new().build(&program).code, simplifier.simplified())
    }

    #[test]
    fn simplifies_assign_then_return() {
        let (output, count) = run_simplify("var r = (a(), x = y, x);");
        assert_eq!(count, 1, "Should simplify 1 sequence, got: {count}");
        assert!(
            output.contains("(a(), x = y)"),
            "Should drop trailing identifier, got: {output}"
        );
        assert!(
            !output.contains(", x)"),
            "Should not have trailing ', x)', got: {output}"
        );
    }

    #[test]
    fn preserves_non_matching() {
        let (output, count) = run_simplify("var r = (a(), x = y, z);");
        assert_eq!(count, 0, "Should not simplify when names differ, got: {count}");
        assert!(output.contains("x = y, z"), "Should preserve original, got: {output}");
    }

    #[test]
    fn simplifies_in_return() {
        let (output, count) = run_simplify("function f() { return Ot.pop(), jC9 = Jc9, jC9; }");
        assert_eq!(count, 1, "Should simplify 1 sequence, got: {count}");
        assert!(
            output.contains("Ot.pop(), jC9 = Jc9"),
            "Should keep assignment, got: {output}"
        );
        assert!(!output.contains(", jC9;"), "Should drop trailing jC9, got: {output}");
    }
}
