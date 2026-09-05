//! Common Subexpression Elimination (CSE)
//!
//! Identifies duplicate expressions and tracks them for potential optimization.
//! Currently detects common subexpressions; full rewriting deferred to future work.

mod clone_ast;
mod stringifier;
#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Program, Statement};
use oxc_semantic::SemanticBuilder;

use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashMap;

use crate::oxc_opts::state::OptimizationState;

pub type Ctx<'a> = TraverseCtx<'a, OptimizationState>;

pub struct CommonSubexpressionElimination {
    changed: bool,
    expr_to_var: FxHashMap<String, String>,
    var_counter: usize,
}

impl CommonSubexpressionElimination {
    #[must_use]
    pub fn new() -> Self {
        Self {
            changed: false,
            expr_to_var: FxHashMap::default(),
            var_counter: 0,
        }
    }

    pub fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        self.expr_to_var.clear();
        self.var_counter = 0;

        let state = OptimizationState::new();
        let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, allocator);

        traverse_mut_with_ctx(self, program, &mut ctx);

        self.changed
    }

    #[allow(dead_code)]
    fn generate_temp_var(&mut self) -> String {
        let var_name = format!("__cse_temp_{}", self.var_counter);
        self.var_counter = self.var_counter.saturating_add(1);
        var_name
    }

    fn process_block_for_cse<'a>(&mut self, statements: &mut oxc_allocator::Vec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let mut new_statements = oxc_allocator::Vec::new_in(&ctx.ast);
        let mut expr_map: FxHashMap<String, String> = FxHashMap::default();

        for stmt in statements.iter() {
            match stmt {
                Statement::ExpressionStatement(expr_stmt) => {
                    if let Some(expr_str) = self.expression_to_string(&expr_stmt.expression) {
                        if self.is_complex_expression(&expr_stmt.expression) && expr_map.contains_key(&expr_str) {
                            self.changed = true;
                            continue;
                        } else if self.is_complex_expression(&expr_stmt.expression) {
                            expr_map.insert(expr_str, String::new());
                        }
                    }
                    new_statements.push(self.clone_statement(stmt, ctx));
                }
                Statement::VariableDeclaration(var_decl) => {
                    for declarator in &var_decl.declarations {
                        if let Some(init) = &declarator.init
                            && let Some(expr_str) = self.expression_to_string(init)
                            && self.is_complex_expression(init)
                        {
                            use std::collections::hash_map::Entry;
                            match expr_map.entry(expr_str) {
                                Entry::Occupied(_) => {
                                    self.changed = true;
                                }
                                Entry::Vacant(e) => {
                                    if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
                                        e.insert(ident.name.to_string());
                                    }
                                }
                            }
                        }
                    }
                    new_statements.push(self.clone_statement(stmt, ctx));
                }
                _ => {
                    new_statements.push(self.clone_statement(stmt, ctx));
                }
            }
        }

        *statements = new_statements;
    }
}

impl Default for CommonSubexpressionElimination {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, OptimizationState> for CommonSubexpressionElimination {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::BlockStatement(block) = stmt {
            self.process_block_for_cse(&mut block.body, ctx);
        }
    }
}
