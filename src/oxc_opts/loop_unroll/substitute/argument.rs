use oxc_ast::ast::{Argument, NumberBase};
use oxc_span::SPAN;

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
                if ident.name == loop_var {
                    Some(Argument::new_numeric_literal(
                        SPAN,
                        value as f64,
                        None,
                        NumberBase::Decimal,
                        ctx,
                    ))
                } else {
                    Some(Argument::new_identifier(SPAN, ident.name.as_str(), ctx))
                }
            }
            Argument::NumericLiteral(lit) => {
                Some(Argument::new_numeric_literal(SPAN, lit.value, lit.raw, lit.base, ctx))
            }
            Argument::StringLiteral(lit) => Some(Argument::new_string_literal(SPAN, lit.value.as_str(), None, ctx)),
            Argument::BooleanLiteral(lit) => Some(Argument::new_boolean_literal(SPAN, lit.value, ctx)),
            Argument::NullLiteral(_) => Some(Argument::new_null_literal(SPAN, ctx)),
            Argument::BinaryExpression(bin) => {
                let left = self.substitute_in_expression(&bin.left, loop_var, value, ctx)?;
                let right = self.substitute_in_expression(&bin.right, loop_var, value, ctx)?;
                Some(Argument::new_binary_expression(SPAN, left, bin.operator, right, ctx))
            }
            Argument::CallExpression(call) => {
                let new_callee = self.substitute_in_expression(&call.callee, loop_var, value, ctx)?;
                let mut new_args = oxc_allocator::Vec::new_in(&ctx.ast);
                for a in &call.arguments {
                    if let Argument::SpreadElement(_) = a {
                        return None;
                    }
                    new_args.push(self.substitute_in_argument(a, loop_var, value, ctx)?);
                }
                Some(Argument::new_call_expression(
                    SPAN,
                    new_callee,
                    None,
                    new_args,
                    call.optional,
                    ctx,
                ))
            }
            _ => None,
        }
    }
}
