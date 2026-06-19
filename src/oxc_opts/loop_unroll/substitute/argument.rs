use oxc_ast::ast::{
    Argument, BinaryExpression, BooleanLiteral, CallExpression, IdentifierReference, NullLiteral, NumberBase,
    NumericLiteral, StringLiteral,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use std::cell::Cell;

use super::super::{Ctx, LoopUnroller};

impl LoopUnroller {
    pub(super) fn substitute_in_argument<'a>(
        &self,
        arg: &Argument<'a>,
        loop_var: &str,
        value: i64,
        ctx: &mut Ctx<'a>,
    ) -> Option<Argument<'a>> {
        match arg {
            Argument::Identifier(ident) => {
                if ident.name.as_str() == loop_var {
                    Some(Argument::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        value: value as f64,
                        raw: None,
                        base: NumberBase::Decimal,
                    })))
                } else {
                    Some(Argument::Identifier(ctx.ast.alloc(IdentifierReference {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        name: ctx.ast.ident(ident.name.as_str()),
                        reference_id: None.into(),
                    })))
                }
            }
            Argument::NumericLiteral(lit) => Some(Argument::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: lit.value,
                raw: lit.raw,
                base: lit.base,
            }))),
            Argument::StringLiteral(lit) => Some(Argument::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: ctx.ast.str(lit.value.as_str()),
                raw: None,
                lone_surrogates: false,
            }))),
            Argument::BooleanLiteral(lit) => Some(Argument::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: lit.value,
            }))),
            Argument::NullLiteral(_) => Some(Argument::NullLiteral(ctx.ast.alloc(NullLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }))),
            Argument::BinaryExpression(bin) => {
                let left = self.substitute_in_expression(&bin.left, loop_var, value, ctx)?;
                let right = self.substitute_in_expression(&bin.right, loop_var, value, ctx)?;
                Some(Argument::BinaryExpression(ctx.ast.alloc(BinaryExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    left,
                    operator: bin.operator,
                    right,
                })))
            }
            Argument::CallExpression(call) => {
                let new_callee = self.substitute_in_expression(&call.callee, loop_var, value, ctx)?;
                let mut new_args = ctx.ast.vec();
                for a in &call.arguments {
                    if let Argument::SpreadElement(_) = a {
                        return None;
                    }
                    let new_a = self.substitute_in_argument(a, loop_var, value, ctx)?;
                    new_args.push(new_a);
                }
                Some(Argument::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee: new_callee,
                    type_arguments: None,
                    arguments: new_args,
                    optional: call.optional,
                    pure: false,
                })))
            }
            _ => None,
        }
    }
}
