use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{BlockStatement, FunctionBody, Program};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod helpers;

use helpers::{has_iife_candidate, try_unwrap_assigned, try_unwrap_standalone};

pub struct IifeUnwrap {
    unwrapped_count: usize,
}
counter_pass!(IifeUnwrap, unwrapped_count, unwrapped_count);

impl<'a> Traverse<'a, DeobfuscateState> for IifeUnwrap {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = program.body.iter().any(|s| has_iife_candidate(s));
        if !has_candidate {
            return;
        }

        let mut new_body = ArenaVec::new_in(ctx);
        for stmt in &program.body {
            if try_unwrap_standalone(stmt, &mut new_body, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            if try_unwrap_assigned(stmt, &mut new_body, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        program.body = new_body;
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = block.body.iter().any(|s| has_iife_candidate(s));
        if !has_candidate {
            return;
        }

        let mut new_body = ArenaVec::new_in(ctx);
        for stmt in &block.body {
            if try_unwrap_standalone(stmt, &mut new_body, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            if try_unwrap_assigned(stmt, &mut new_body, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            new_body.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        block.body = new_body;
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        let has_candidate = body.statements.iter().any(|s| has_iife_candidate(s));
        if !has_candidate {
            return;
        }

        let mut new_stmts = ArenaVec::new_in(ctx);
        for stmt in &body.statements {
            if try_unwrap_standalone(stmt, &mut new_stmts, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            if try_unwrap_assigned(stmt, &mut new_stmts, ctx) {
                self.unwrapped_count = self.unwrapped_count.wrapping_add(1);
                continue;
            }
            new_stmts.push(stmt.clone_in_with_semantic_ids(ctx.ast.allocator()));
        }
        body.statements = new_stmts;
    }
}

#[cfg(test)]
mod tests;
