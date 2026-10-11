use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{Expression, Statement};
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use super::CaseBodyMap;
use crate::ast_deobfuscate::dispatcher_detector::DispatcherMap;
use crate::ast_deobfuscate::state::DeobfuscateState;

mod plan;
mod replacements;
mod resolver;

use plan::{Shape, plan};
use replacements::{clone_statements, make_iife};
use resolver::{CallResolution, resolve_call};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub(super) struct DispatcherMeta {
    args_param: String,
}

pub struct CffUnflattener<'a> {
    dispatchers: DispatcherMap,
    case_bodies: CaseBodyMap<'a>,
    meta: FxHashMap<String, DispatcherMeta>,
    inlined: usize,
}

impl<'a> CffUnflattener<'a> {
    #[must_use]
    pub fn new(dispatchers: DispatcherMap, case_bodies: CaseBodyMap<'a>) -> Self {
        let meta: FxHashMap<String, DispatcherMeta> = dispatchers
            .iter()
            .map(|(name, info)| {
                (
                    name.clone(),
                    DispatcherMeta {
                        args_param: info.args_param.clone(),
                    },
                )
            })
            .collect();
        Self {
            dispatchers,
            case_bodies,
            meta,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }
}

impl<'a> CffUnflattener<'a> {
    fn resolve_call<'b>(&'b self, call: &'b oxc_ast::ast::CallExpression<'a>) -> Option<CallResolution<'a, 'b>> {
        resolve_call(&self.dispatchers, &self.case_bodies, &self.meta, call)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for CffUnflattener<'a> {
    fn exit_statements(&mut self, stmts: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let mut i = 0;
        while i < stmts.len() {
            let Statement::ExpressionStatement(es) = &stmts[i] else {
                i += 1;
                continue;
            };
            let Expression::CallExpression(call) = &es.expression else {
                i += 1;
                continue;
            };
            let Some(resolution) = self.resolve_call(call) else {
                i += 1;
                continue;
            };
            let Some(shape) = plan(&resolution, ctx.current_scope_id(), ctx.scoping()) else {
                i += 1;
                continue;
            };
            let replacement: Vec<Statement<'a>> = match shape {
                Shape::Splice => clone_statements(&resolution.body.stmts, ctx).collect(),
                Shape::Iife { bind_args } => {
                    let iife = make_iife(
                        &resolution.meta.args_param,
                        &resolution.body.stmts,
                        bind_args.then_some(resolution.args),
                        ctx,
                    );
                    vec![Statement::new_expression_statement(SPAN, iife, ctx)]
                }
            };

            if self.inlined < 10 {
                eprintln!("[CFF] inlining call site (stmt-level)");
            }
            self.inlined += 1;

            let count = replacement.len();
            stmts.splice(i..=i, replacement);
            i += count;
        }
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if matches!(ctx.parent(), Ancestor::ExpressionStatementExpression(_)) {
            return;
        }
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Some(resolution) = self.resolve_call(call) else {
            return;
        };
        let Some(shape) = plan(&resolution, ctx.current_scope_id(), ctx.scoping()) else {
            return;
        };
        let bind_args = match shape {
            Shape::Splice => false,
            Shape::Iife { bind_args } => bind_args,
        };
        let replacement = make_iife(
            &resolution.meta.args_param,
            &resolution.body.stmts,
            bind_args.then_some(resolution.args),
            ctx,
        );

        if self.inlined < 10 {
            eprintln!("[CFF] inlining call site (expr-level)");
        }
        self.inlined += 1;

        *expr = replacement;
    }
}
