use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{Expression, MemberExpression};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ArrayUnpacker {
    changed: bool,
}

impl ArrayUnpacker {
    fn try_unpack<'a>(&mut self, member: &MemberExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let MemberExpression::ComputedMemberExpression(computed) = member else {
            return None;
        };

        let Expression::ArrayExpression(array) = &computed.object else {
            return None;
        };

        let Expression::NumericLiteral(index_lit) = &computed.expression else {
            return None;
        };

        let index = index_lit.value as usize;

        if index >= array.elements.len() {
            return None;
        }

        let element = &array.elements[index];

        if element.is_elision() || element.is_spread() {
            return None;
        }

        let expr = element.to_expression();

        eprintln!("[AST] Unpacking array access at index {index}");
        self.changed = true;

        Some(Self::clone_expression(expr, ctx))
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator())
    }
}

flag_pass!(ArrayUnpacker, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for ArrayUnpacker {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::ComputedMemberExpression(member) = expr {
            let member_expr =
                MemberExpression::ComputedMemberExpression(member.clone_in_with_semantic_ids(ctx.ast.allocator()));
            if let Some(unpacked) = self.try_unpack(&member_expr, ctx) {
                *expr = unpacked;
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
    fn test_unpack_array_identifier() {
        let output = run_array_unpack("var x = [foo, bar][0];");
        eprintln!("Output: {output}");
        assert!(
            output.contains("= foo") || output.contains("=foo"),
            "Should unpack to identifier, got: {output}"
        );
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
