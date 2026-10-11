use oxc_ast::ast::{CallExpression, Expression, IdentifierReference};

use crate::ast_deobfuscate::state::DispatcherInfo;

use super::{Ctx, DispatcherInliner};

impl DispatcherInliner {
    fn dispatcher_of(&self, object: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> Option<&DispatcherInfo> {
        let symbol = ctx.scoping().get_reference(object.reference_id()).symbol_id()?;
        self.detected_dispatchers.get(&symbol)
    }

    pub(super) fn try_inline_dispatcher_call<'a>(
        &self,
        call: &CallExpression<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<Expression<'a>> {
        // Try computed member expression first: d["key"]()
        if let Some(result) = self.try_inline_computed_member(call, ctx) {
            return Some(result);
        }

        // Try static member expression: d.key()
        if let Some(result) = self.try_inline_static_member(call, ctx) {
            return Some(result);
        }

        None
    }

    fn try_inline_computed_member<'a>(&self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::ComputedMemberExpression(member) = &call.callee else {
            return None;
        };

        let Expression::Identifier(object) = &member.object else {
            return None;
        };
        let obj_name = object.name.as_str();
        let dispatcher = self.dispatcher_of(object, ctx)?;

        // Support string literal keys: d["key"]()
        let key = match &member.expression {
            Expression::StringLiteral(lit) => lit.value.as_str(),
            _ => return None,
        };

        let func_info = dispatcher.functions.get(key)?;
        let return_value = func_info.return_value.as_ref()?;

        eprintln!("[AST] Inlining computed: {obj_name}[\"{key}\"]() → {return_value:?}");

        Some(Self::create_expression_from_return_value(return_value, ctx))
    }

    fn try_inline_static_member<'a>(&self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };

        let Expression::Identifier(object) = &member.object else {
            return None;
        };
        let obj_name = object.name.as_str();
        let dispatcher = self.dispatcher_of(object, ctx)?;

        // Property must be an identifier: d.key (not d["key"])
        let key = member.property.name.as_str();

        let func_info = dispatcher.functions.get(key)?;
        let return_value = func_info.return_value.as_ref()?;

        eprintln!("[AST] Inlining static: {obj_name}.{key}() → {return_value:?}");

        Some(Self::create_expression_from_return_value(return_value, ctx))
    }
}
