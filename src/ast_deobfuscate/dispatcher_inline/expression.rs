use oxc_ast::ast::{Expression, Ident, Str};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;

use crate::ast_deobfuscate::state::ReturnValue;

use super::{Ctx, DispatcherInliner};

impl DispatcherInliner {
    pub(super) fn create_expression_from_return_value<'a>(
        return_value: &ReturnValue,
        ctx: &mut Ctx<'a>,
    ) -> Expression<'a> {
        match return_value {
            ReturnValue::Number(n) => Expression::new_numeric_literal(SPAN, *n, None, NumberBase::Decimal, ctx),
            ReturnValue::String(s) => {
                Expression::new_string_literal(SPAN, Str::from_str_in(s.as_str(), ctx), None, ctx)
            }
            ReturnValue::Bool(b) => Expression::new_boolean_literal(SPAN, *b, ctx),
            ReturnValue::Null => Expression::new_null_literal(SPAN, ctx),
            ReturnValue::Identifier(name) => ctx.create_unbound_ident_expr(
                SPAN,
                Ident::from_str_in(name.as_str(), &ctx.ast),
                oxc_syntax::reference::ReferenceFlags::Read,
            ),
        }
    }
}
