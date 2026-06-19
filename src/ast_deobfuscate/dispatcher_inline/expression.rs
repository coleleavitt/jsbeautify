use oxc_ast::ast::{BooleanLiteral, Expression, IdentifierReference, NullLiteral, NumericLiteral, StringLiteral};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::number::NumberBase;
use std::cell::Cell;

use crate::ast_deobfuscate::state::ReturnValue;

use super::{Ctx, DispatcherInliner};

impl DispatcherInliner {
    pub(super) fn create_expression_from_return_value<'a>(
        return_value: &ReturnValue,
        ctx: &mut Ctx<'a>,
    ) -> Expression<'a> {
        match return_value {
            ReturnValue::Number(n) => Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *n,
                raw: None,
                base: NumberBase::Decimal,
            })),
            ReturnValue::String(s) => Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: ctx.ast.str(s.as_str()),
                raw: None,
                lone_surrogates: false,
            })),
            ReturnValue::Bool(b) => Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *b,
            })),
            ReturnValue::Null => Expression::NullLiteral(ctx.ast.alloc(NullLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            })),
            ReturnValue::Identifier(name) => Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(name.as_str()),
                reference_id: None.into(),
            })),
        }
    }
}
