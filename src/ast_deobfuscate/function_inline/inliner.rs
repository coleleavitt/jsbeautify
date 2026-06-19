use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    Argument, BinaryExpression, BinaryOperator, BooleanLiteral, CallExpression, EmptyStatement, Expression,
    IdentifierReference, NullLiteral, NumericLiteral, Statement, StringLiteral, UnaryExpression, UnaryOperator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::number::NumberBase;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{BinaryOp, InlinableFunction, ReturnExpr, UnaryOp};

pub struct FunctionInliner {
    functions: FxHashMap<String, InlinableFunction>,
    changed: bool,
}

impl FunctionInliner {
    #[must_use]
    pub const fn new(functions: FxHashMap<String, InlinableFunction>) -> Self {
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
        let name = if let Expression::Identifier(ident) = &call.callee {
            ident.name.as_str()
        } else {
            return None;
        };

        let func = self.functions.get(name)?;

        if call.arguments.len() != func.params.len() {
            return None;
        }

        let return_expr = func.return_expression.as_ref()?;

        let args: Vec<_> = call.arguments.iter().filter_map(|arg| arg.as_expression()).collect();

        if args.len() != func.params.len() {
            return None;
        }

        eprintln!("[AST] Inlining call to function: {name}");
        self.changed = true;

        Some(Self::build_expression(return_expr, &args, ctx))
    }

    fn build_expression<'a>(ret_expr: &ReturnExpr, args: &[&Expression<'a>], ctx: &mut Ctx<'a>) -> Expression<'a> {
        match ret_expr {
            ReturnExpr::ParamRef(idx) => {
                if *idx < args.len() {
                    Self::clone_expression(args[*idx], ctx)
                } else {
                    Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        name: ctx.ast.ident("undefined"),
                        reference_id: Cell::default(),
                    }))
                }
            }
            ReturnExpr::Identifier(name) => Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(name),
                reference_id: Cell::default(),
            })),
            ReturnExpr::Number(val) => Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *val,
                raw: Some(ctx.ast.str(&val.to_string())),
                base: NumberBase::Decimal,
            })),
            ReturnExpr::String(s) => Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: ctx.ast.str(s),
                raw: None,
                lone_surrogates: false,
            })),
            ReturnExpr::Boolean(b) => Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *b,
            })),
            ReturnExpr::Null => Expression::NullLiteral(ctx.ast.alloc(NullLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            })),
            ReturnExpr::Binary { left, op, right } => {
                let left_expr = Self::build_expression(left, args, ctx);
                let right_expr = Self::build_expression(right, args, ctx);
                let operator = match op {
                    BinaryOp::Add => BinaryOperator::Addition,
                    BinaryOp::Sub => BinaryOperator::Subtraction,
                    BinaryOp::Mul => BinaryOperator::Multiplication,
                    BinaryOp::Div => BinaryOperator::Division,
                    BinaryOp::Mod => BinaryOperator::Remainder,
                    BinaryOp::BitAnd | BinaryOp::And => BinaryOperator::BitwiseAnd,
                    BinaryOp::BitOr | BinaryOp::Or => BinaryOperator::BitwiseOR,
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
                Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    left: left_expr,
                    operator,
                    right: right_expr,
                }))
            }
            ReturnExpr::Unary { op, arg } => {
                let arg_expr = Self::build_expression(arg, args, ctx);
                let operator = match op {
                    UnaryOp::Not => UnaryOperator::LogicalNot,
                    UnaryOp::Neg => UnaryOperator::UnaryNegation,
                    UnaryOp::Plus => UnaryOperator::UnaryPlus,
                    UnaryOp::BitNot => UnaryOperator::BitwiseNot,
                };
                Expression::UnaryExpression(ctx.ast.alloc(UnaryExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    operator,
                    argument: arg_expr,
                }))
            }
            ReturnExpr::Call {
                callee,
                args: call_args,
            } => {
                let mut arguments = ctx.ast.vec();
                for arg in call_args {
                    let expr = Self::build_expression(arg, args, ctx);
                    arguments.push(Argument::from(expr));
                }
                Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee: Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        name: ctx.ast.ident(callee),
                        reference_id: Cell::default(),
                    })),
                    arguments,
                    optional: false,
                    type_arguments: None,
                    pure: false,
                }))
            }
        }
    }

    fn clone_expression<'a>(expr: &Expression<'a>, ctx: &mut Ctx<'a>) -> Expression<'a> {
        expr.clone_in_with_semantic_ids(ctx.ast.allocator)
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
        {
            let name = id.name.as_str();
            if self.functions.contains_key(name) {
                eprintln!("[AST] Removing inlined function declaration: {name}");
                self.changed = true;
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
            }
        }
    }
}
