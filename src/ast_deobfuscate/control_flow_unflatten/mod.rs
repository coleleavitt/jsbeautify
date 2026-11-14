//! Control flow unflattening pass
//!
//! Restores the dispatch loops javascript-obfuscator emits for flattened
//! blocks:
//! ```js
//! // Obfuscated (flattened)
//! var _flow = "2|0|1".split('|'), _i = 0;
//! while (true) {
//!     switch (_flow[_i++]) {
//!         case "0": console.log("second"); continue;
//!         case "1": console.log("third"); break;
//!         case "2": console.log("first"); continue;
//!     }
//!     break;
//! }
//!
//! // Deobfuscated (restored)
//! console.log("first");
//! console.log("second");
//! console.log("third");
//! ```
//!
//! The rewrite runs on every statement list (program and function bodies,
//! blocks) and replaces a loop only together with the declarations of its
//! sequence and index variables, and only when the loop's discriminant is the
//! sole reference to each of them. See [`analysis::plan_loop`] for the full
//! set of conditions.

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod analysis;

pub struct ControlFlowUnflattener {
    rewrites: usize,
    script: bool,
}

impl ControlFlowUnflattener {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rewrites: 0,
            script: false,
        }
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }

    #[must_use]
    pub const fn changed(&self) -> bool {
        self.rewrites > 0
    }
}

impl Default for ControlFlowUnflattener {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ControlFlowUnflattener {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.script = program.source_type.is_script();
    }

    fn exit_statements(&mut self, stmts: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !stmts
            .iter()
            .any(|stmt| matches!(stmt, Statement::WhileStatement(_) | Statement::ForStatement(_)))
        {
            return;
        }
        let declarators = analysis::declarators(stmts);
        let plans: Vec<_> = (0..stmts.len())
            .filter_map(|index| analysis::plan_loop(stmts, index, &declarators, self.script, ctx))
            .collect();
        if plans.is_empty() {
            return;
        }

        let mut bodies = FxHashMap::default();
        let mut dropped: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
        for plan in plans {
            deob_trace!("[AST] Unflattened control-flow loop: {} statements", plan.body.len());
            for (stmt, declarator) in plan.declarators {
                dropped.entry(stmt).or_default().push(declarator);
            }
            bodies.insert(plan.loop_index, plan.body);
            self.rewrites += 1;
        }

        let old = std::mem::replace(stmts, OxcVec::new_in(ctx));
        for (index, mut stmt) in old.into_iter().enumerate() {
            if let Some(body) = bodies.remove(&index) {
                stmts.extend(body);
                continue;
            }
            if let Some(declarators) = dropped.get(&index)
                && let Statement::VariableDeclaration(decl) = &mut stmt
            {
                let mut position = 0;
                decl.declarations.retain(|_| {
                    let keep = !declarators.contains(&position);
                    position += 1;
                    keep
                });
                if decl.declarations.is_empty() {
                    continue;
                }
            }
            stmts.push(stmt);
        }
    }
}

#[cfg(test)]
mod tests;
