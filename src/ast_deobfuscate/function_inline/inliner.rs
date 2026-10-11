use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BinaryOperator, CallExpression, Expression, Statement, UnaryOperator};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::rename_guards::{binds_global, is_annex_b_function};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{BinaryOp, InlinableFunction, ReturnExpr, UnaryOp};

pub struct FunctionInliner {
    functions: FxHashMap<SymbolId, InlinableFunction>,
    changed: bool,
}

impl FunctionInliner {
    #[must_use]
    pub const fn new(functions: FxHashMap<SymbolId, InlinableFunction>) -> Self {
        Self {
            functions,
            changed: false,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn try_inline_call<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let symbol = ctx.scoping().get_reference(ident.reference_id()).symbol_id()?;
        let func = self.functions.get(&symbol)?;
        let args = call
            .arguments
            .iter()
            .map(|arg| arg.as_expression())
            .collect::<Option<Vec<_>>>()?;

        eprintln!("[AST] Inlining call to function: {}", func.name);
        self.changed = true;
        Some(Self::build_expression(&func.return_expression, &args, ctx))
    }

    fn build_expression<'a>(ret_expr: &ReturnExpr, args: &[&Expression<'a>], ctx: &mut Ctx<'a>) -> Expression<'a> {
        match ret_expr {
            ReturnExpr::ParamRef(idx) => Self::clone_expression(args[*idx], ctx),
            ReturnExpr::Identifier { name, .. } => {
                Expression::new_identifier(SPAN, ctx.allocator().alloc_str(name), ctx)
            }
            ReturnExpr::Number(val) => Expression::new_numeric_literal(SPAN, *val, None, NumberBase::Decimal, ctx),
            ReturnExpr::String(value) => {
                Expression::new_string_literal(SPAN, ctx.allocator().alloc_str(value), None, ctx)
            }
            ReturnExpr::Boolean(value) => Expression::new_boolean_literal(SPAN, *value, ctx),
            ReturnExpr::Null => Expression::new_null_literal(SPAN, ctx),
            ReturnExpr::Binary { left, op, right } => {
                let left = Self::build_expression(left, args, ctx);
                let right = Self::build_expression(right, args, ctx);
                let operator = match op {
                    BinaryOp::Add => BinaryOperator::Addition,
                    BinaryOp::Sub => BinaryOperator::Subtraction,
                    BinaryOp::Mul => BinaryOperator::Multiplication,
                    BinaryOp::Div => BinaryOperator::Division,
                    BinaryOp::Mod => BinaryOperator::Remainder,
                    BinaryOp::BitAnd => BinaryOperator::BitwiseAnd,
                    BinaryOp::BitOr => BinaryOperator::BitwiseOR,
                    BinaryOp::BitXor => BinaryOperator::BitwiseXOR,
                    BinaryOp::Eq => BinaryOperator::Equality,
                    BinaryOp::NotEq => BinaryOperator::Inequality,
                    BinaryOp::StrictEq => BinaryOperator::StrictEquality,
                    BinaryOp::StrictNotEq => BinaryOperator::StrictInequality,
                    BinaryOp::Lt => BinaryOperator::LessThan,
                    BinaryOp::Lte => BinaryOperator::LessEqualThan,
                    BinaryOp::Gt => BinaryOperator::GreaterThan,
                    BinaryOp::Gte => BinaryOperator::GreaterEqualThan,
                };
                Expression::new_binary_expression(SPAN, left, operator, right, ctx)
            }
            ReturnExpr::Unary { op, arg } => {
                let argument = Self::build_expression(arg, args, ctx);
                let operator = match op {
                    UnaryOp::Not => UnaryOperator::LogicalNot,
                    UnaryOp::Neg => UnaryOperator::UnaryNegation,
                    UnaryOp::Plus => UnaryOperator::UnaryPlus,
                    UnaryOp::BitNot => UnaryOperator::BitwiseNot,
                };
                Expression::new_unary_expression(SPAN, operator, argument, ctx)
            }
        }
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator())
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for FunctionInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline_call(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
            && id
                .symbol_id
                .get()
                .is_some_and(|symbol| self.functions.contains_key(&symbol))
            && !binds_global(ctx, id)
            && !is_annex_b_function(ctx, func)
        {
            eprintln!("[AST] Removing inlined function declaration: {}", id.name);
            self.changed = true;
            *stmt = Statement::new_empty_statement(SPAN, ctx);
        }
    }
}
