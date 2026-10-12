use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{ComputedMemberExpression, Expression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ArrayUnpacker {
    rewrites: usize,
}

/// Literals whose evaluation has no side effects. Only these may be selected
/// or discarded: any other element could be observable when dropped, or (as a
/// callee) would change `this` once taken out of the array.
const fn is_primitive_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    )
}

impl ArrayUnpacker {
    fn try_unpack<'a>(&mut self, member: &ComputedMemberExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::ArrayExpression(array) = &member.object else {
            return None;
        };
        let Expression::NumericLiteral(index_lit) = &member.expression else {
            return None;
        };
        let index = index_lit.value;
        if index.fract() != 0.0 || index < 0.0 || index >= array.elements.len() as f64 {
            return None;
        }
        let elements = array
            .elements
            .iter()
            .map(|element| element.as_expression().filter(|expr| is_primitive_literal(expr)))
            .collect::<Option<Vec<_>>>()?;

        deob_trace!("[AST] Unpacking array access at index {index}");
        self.rewrites += 1;
        Some(elements[index as usize].clone_in_with_semantic_ids(ctx.ast.allocator()))
    }
}

counter_pass!(ArrayUnpacker, rewrites, rewrites);

impl<'a> Traverse<'a, DeobfuscateState> for ArrayUnpacker {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::ComputedMemberExpression(member) = expr
            && let Some(unpacked) = self.try_unpack(member, ctx)
        {
            *expr = unpacked;
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

    fn run_array_unpack(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut unpacker = ArrayUnpacker::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut unpacker, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_unpack_array_first_element() {
        let output = run_array_unpack("var x = [\"a\", \"b\"][0];");
        eprintln!("Output: {output}");
        assert!(
            output.contains("\"a\"") && !output.contains("[\"a\""),
            "Should unpack to first element, got: {output}"
        );
    }

    #[test]
    fn test_unpack_array_second_element() {
        let output = run_array_unpack("var x = [1, 2, 3][1];");
        eprintln!("Output: {output}");
        assert!(
            output.contains("= 2") || output.contains("=2"),
            "Should unpack to second element, got: {output}"
        );
    }

    #[test]
    fn test_preserve_non_literal_elements() {
        for code in ["x = [foo, bar][0];\n", "x = [1, f()][0];\n", "x = [o.m][0]();\n"] {
            assert_eq!(run_array_unpack(code), code);
        }
    }

    #[test]
    fn test_preserve_non_index_keys() {
        for code in [
            "v = [1][-1];\n",
            "v = [1][1.5];\n",
            "v = [...a, 1][1];\n",
            "v = [1, ...a][0];\n",
            "v = [, 1][0];\n",
        ] {
            assert_eq!(run_array_unpack(code), code);
        }
    }

    #[test]
    fn test_preserve_out_of_bounds() {
        let output = run_array_unpack("var x = [1, 2][5];");
        eprintln!("Output: {output}");
        assert!(
            output.contains("[1, 2][5]") || output.contains("[1,2][5]"),
            "Should preserve out of bounds access, got: {output}"
        );
    }
}
