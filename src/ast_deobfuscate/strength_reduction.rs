use oxc_allocator::CloneIn;
use oxc_ast::ast::{BinaryExpression, BinaryOperator, Expression, NumericLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::number::NumberBase;
use oxc_traverse::{Traverse, TraverseCtx};
use std::cell::Cell;

use crate::ast_deobfuscate::pass::flag_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct StrengthReducer {
    changed: bool,
}

impl StrengthReducer {
    fn try_reduce_multiply<'a>(&mut self, binary: &BinaryExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Multiplication {
            return None;
        }

        let n = Self::extract_u32(&binary.right)?;
        if !Self::is_power_of_two(n) {
            return None;
        }

        let shift_amount = (n as f64).log2() as u32;

        eprintln!("[AST] Reducing x * {n} to x << {shift_amount}");

        self.changed = true;
        Some(Self::make_shift_left(&binary.left, shift_amount, ctx))
    }

    fn try_reduce_divide<'a>(&mut self, binary: &BinaryExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Division {
            return None;
        }

        let n = Self::extract_u32(&binary.right)?;
        if !Self::is_power_of_two(n) {
            return None;
        }

        let shift_amount = (n as f64).log2() as u32;

        eprintln!("[AST] Reducing x / {n} to x >> {shift_amount}");

        self.changed = true;
        Some(Self::make_shift_right(&binary.left, shift_amount, ctx))
    }

    fn try_reduce_modulo<'a>(&mut self, binary: &BinaryExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        if binary.operator != BinaryOperator::Remainder {
            return None;
        }

        let n = Self::extract_u32(&binary.right)?;
        if !Self::is_power_of_two(n) {
            return None;
        }

        let mask = n - 1;

        eprintln!("[AST] Reducing x % {n} to x & {mask}");

        self.changed = true;
        Some(Self::make_bitwise_and(&binary.left, mask, ctx))
    }

    fn extract_u32(expr: &Expression<'_>) -> Option<u32> {
        if let Expression::NumericLiteral(num) = expr {
            let val = num.value;
            if val >= 0.0 && val <= u32::MAX as f64 && val.fract() == 0.0 {
                return Some(val as u32);
            }
        }
        None
    }

    const fn is_power_of_two(n: u32) -> bool {
        n > 0 && (n & (n.wrapping_sub(1))) == 0
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
    }

    fn make_number<'a>(val: u32, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: val as f64,
            raw: Some(ctx.ast.str(&val.to_string())),
            base: NumberBase::Decimal,
        }))
    }

    fn make_shift_left<'a>(left: &Expression<'a>, shift: u32, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            left: Self::clone_expression(left, ctx),
            operator: BinaryOperator::ShiftLeft,
            right: Self::make_number(shift, ctx),
        }))
    }

    fn make_shift_right<'a>(left: &Expression<'a>, shift: u32, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            left: Self::clone_expression(left, ctx),
            operator: BinaryOperator::ShiftRight,
            right: Self::make_number(shift, ctx),
        }))
    }

    fn make_bitwise_and<'a>(left: &Expression<'a>, mask: u32, ctx: &mut Ctx<'a>) -> Expression<'a> {
        Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            left: Self::clone_expression(left, ctx),
            operator: BinaryOperator::BitwiseAnd,
            right: Self::make_number(mask, ctx),
        }))
    }
}

flag_pass!(StrengthReducer, changed, has_changed);

impl<'a> Traverse<'a, DeobfuscateState> for StrengthReducer {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::BinaryExpression(binary) => self
                .try_reduce_multiply(binary, ctx)
                .or_else(|| self.try_reduce_divide(binary, ctx))
                .or_else(|| self.try_reduce_modulo(binary, ctx)),
            _ => None,
        };

        if let Some(new_expr) = replacement {
            *expr = new_expr;
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

    fn run_reduce(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut reducer = StrengthReducer::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut reducer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_multiply_by_2() {
        let output = run_reduce("var r = x * 2;");
        assert!(output.contains("<<"), "Should reduce x * 2 to x << 1, got: {output}");
        assert!(output.contains('1'), "Shift amount should be 1, got: {output}");
    }

    #[test]
    fn test_multiply_by_4() {
        let output = run_reduce("var r = x * 4;");
        assert!(output.contains("<<"), "Should reduce x * 4 to x << 2, got: {output}");
        assert!(output.contains('2'), "Shift amount should be 2, got: {output}");
    }

    #[test]
    fn test_multiply_by_8() {
        let output = run_reduce("var r = x * 8;");
        assert!(output.contains("<<"), "Should reduce x * 8 to x << 3, got: {output}");
        assert!(output.contains('3'), "Shift amount should be 3, got: {output}");
    }

    #[test]
    fn test_divide_by_2() {
        let output = run_reduce("var r = x / 2;");
        assert!(output.contains(">>"), "Should reduce x / 2 to x >> 1, got: {output}");
        assert!(output.contains('1'), "Shift amount should be 1, got: {output}");
    }

    #[test]
    fn test_divide_by_4() {
        let output = run_reduce("var r = x / 4;");
        assert!(output.contains(">>"), "Should reduce x / 4 to x >> 2, got: {output}");
        assert!(output.contains('2'), "Shift amount should be 2, got: {output}");
    }

    #[test]
    fn test_modulo_by_2() {
        let output = run_reduce("var r = x % 2;");
        assert!(output.contains('&'), "Should reduce x % 2 to x & 1, got: {output}");
        assert!(output.contains('1'), "Mask should be 1, got: {output}");
    }

    #[test]
    fn test_modulo_by_4() {
        let output = run_reduce("var r = x % 4;");
        assert!(output.contains('&'), "Should reduce x % 4 to x & 3, got: {output}");
        assert!(output.contains('3'), "Mask should be 3, got: {output}");
    }

    #[test]
    fn test_no_reduction_non_power_of_two() {
        let output = run_reduce("var r = x * 3;");
        assert!(output.contains('*'), "Should NOT reduce x * 3, got: {output}");
    }

    #[test]
    fn test_power_of_two() {
        assert!(StrengthReducer::is_power_of_two(1));
        assert!(StrengthReducer::is_power_of_two(2));
        assert!(StrengthReducer::is_power_of_two(4));
        assert!(StrengthReducer::is_power_of_two(8));
        assert!(StrengthReducer::is_power_of_two(16));
        assert!(!StrengthReducer::is_power_of_two(0));
        assert!(!StrengthReducer::is_power_of_two(3));
        assert!(!StrengthReducer::is_power_of_two(5));
    }
}
