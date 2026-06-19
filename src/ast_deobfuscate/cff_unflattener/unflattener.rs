use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{Expression, Statement};
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use super::CaseBodyMap;
use crate::ast_deobfuscate::dispatcher_detector::DispatcherMap;
use crate::ast_deobfuscate::state::DeobfuscateState;

mod replacements;
mod resolver;

use replacements::{make_iife, make_inline_stmts};
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
            let call = match &stmts[i] {
                Statement::ExpressionStatement(es) => {
                    if let Expression::CallExpression(c) = &es.expression {
                        Some(c.as_ref())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(call) = call else {
                i += 1;
                continue;
            };
            let Some((_, _, args_expr, meta, body)) = self.resolve_call(call) else {
                i += 1;
                continue;
            };
            let args_param = meta.args_param.clone();
            let replacement = make_inline_stmts(&args_param, body, args_expr, ctx);

            if self.inlined < 10 {
                eprintln!("[CFF] inlining call site (stmt-level)");
            }
            self.inlined += 1;

            let count = replacement.len();
            stmts.remove(i);
            for (j, s) in replacement.into_iter().enumerate() {
                stmts.insert(i + j, s);
            }
            i += count;
        }
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if matches!(ctx.parent(), Ancestor::ExpressionStatementExpression(_)) {
            return;
        }
        if let Expression::CallExpression(call) = expr {
            let Some((_, _, args_expr, meta, body)) = self.resolve_call(call) else {
                return;
            };
            let args_param = meta.args_param.clone();
            let replacement = make_iife(&args_param, body, args_expr, ctx);

            if self.inlined < 10 {
                eprintln!("[CFF] inlining call site (expr-level)");
            }
            self.inlined += 1;

            *expr = replacement;
        }
    }
}
