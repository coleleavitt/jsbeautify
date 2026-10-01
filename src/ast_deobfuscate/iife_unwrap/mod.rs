use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{BlockStatement, FunctionBody, Program, Statement};
use oxc_syntax::scope::ScopeId;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod guard;
mod helpers;

use guard::{NameCounts, Target, can_splice};
use helpers::{has_iife_candidate, iife_body, try_unwrap_assigned, try_unwrap_standalone};

pub struct IifeUnwrap {
    unwrapped_count: usize,
}
counter_pass!(IifeUnwrap, unwrapped_count, unwrapped_count);

impl IifeUnwrap {
    /// Splice every safely unwrappable arrow IIFE in `stmts` into the list.
    /// Returns `None` when nothing changed.
    fn unwrap_statements<'a>(
        &mut self,
        stmts: &ArenaVec<'a, Statement<'a>>,
        target: Target,
        target_scope: Option<ScopeId>,
        ctx: &mut Ctx<'a>,
    ) -> Option<ArenaVec<'a, Statement<'a>>> {
        if !stmts.iter().any(|s| has_iife_candidate(s)) {
            return None;
        }
        let target_scope = target_scope?;
        let counts = NameCounts::of_statements(stmts);

        let mut new_body = ArenaVec::new_in(ctx);
        let mut changed = false;
        for stmt in stmts {
            let safe = iife_body(stmt).is_some_and(|body| can_splice(body, stmt, &counts, target, target_scope, ctx));
            if safe
                && (try_unwrap_standalone(stmt, &mut new_body, ctx) || try_unwrap_assigned(stmt, &mut new_body, ctx))
            {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                changed = true;
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        changed.then_some(new_body)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for IifeUnwrap {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let target = if program.source_type.is_script() {
            Target::ScriptTop
        } else {
            Target::VarScope
        };
        if let Some(body) = self.unwrap_statements(&program.body, target, program.scope_id.get(), ctx) {
            program.body = body;
        }
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        if let Some(body) = self.unwrap_statements(&block.body, Target::Block, block.scope_id.get(), ctx) {
            block.body = body;
        }
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        // The enclosing function's scope is still current while its body exits.
        let scope = Some(ctx.current_scope_id());
        if let Some(stmts) = self.unwrap_statements(&body.statements, Target::VarScope, scope, ctx) {
            body.statements = stmts;
        }
    }
}

#[cfg(test)]
mod tests;
