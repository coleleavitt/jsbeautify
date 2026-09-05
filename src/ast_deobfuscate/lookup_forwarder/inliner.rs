use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{BindingPattern, CallExpression, Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::collector::LookupForwarderCollector;
use super::types::LookupForwarderInfo;

pub struct LookupForwarderInliner {
    forwarders: FxHashMap<String, LookupForwarderInfo>,
    inlined: usize,
}

impl LookupForwarderInliner {
    #[must_use]
    pub const fn new(forwarders: FxHashMap<String, LookupForwarderInfo>) -> Self {
        Self { forwarders, inlined: 0 }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn try_inline<'a>(&mut self, call: &CallExpression<'a>, ctx: &mut Ctx<'a>) -> Option<Expression<'a>> {
        let Expression::Identifier(ident) = &call.callee else {
            return None;
        };
        let info = self.forwarders.get(ident.name.as_str())?;
        if call.arguments.len() != 1 {
            return None;
        }
        let arg = call.arguments[0].as_expression()?;
        if self.inlined < 10 {
            eprintln!(
                "[AST/lookup-fwd] inline {}(x) -> {}()[x]",
                ident.name.as_str(),
                info.accessor_name
            );
        }
        self.inlined += 1;

        let accessor_callee = Expression::new_identifier(SPAN, ctx.allocator().alloc_str(&info.accessor_name), ctx);
        let accessor_call = Expression::new_call_expression(SPAN, accessor_callee, None, [], false, ctx);
        let index_expr = arg.clone_in_with_semantic_ids(ctx.ast.allocator());
        Some(Expression::new_computed_member_expression(
            SPAN,
            accessor_call,
            index_expr,
            false,
            ctx,
        ))
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LookupForwarderInliner {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(inlined) = self.try_inline(call, ctx)
        {
            *expr = inlined;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(id) = &func.id
            && self.forwarders.contains_key(id.name.as_str())
        {
            eprintln!("[AST/lookup-fwd] remove function {}", id.name.as_str());
            *stmt = Statement::new_empty_statement(SPAN, ctx);
            return;
        }
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_are_forwarders = !decl.declarations.is_empty()
                && decl.declarations.iter().all(|d| {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return false;
                    };
                    let Some(Expression::FunctionExpression(func)) = &d.init else {
                        return false;
                    };
                    self.forwarders.contains_key(id.name.as_str())
                        && LookupForwarderCollector::extract_info(func).is_some()
                });
            if all_are_forwarders {
                eprintln!("[AST/lookup-fwd] remove var declaration");
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
        }
    }
}
