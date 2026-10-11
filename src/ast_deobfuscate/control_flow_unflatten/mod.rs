//! Control flow unflattening pass
//!
//! Detects and restores flattened control flow patterns:
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

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod analysis;
mod types;

pub use types::ControlFlowInfo;
use types::UnflattenAction;

pub struct ControlFlowUnflattener {
    detected_sequences: FxHashMap<String, Vec<String>>,
    detected_index_vars: FxHashMap<String, String>,
    changed: bool,
}

impl ControlFlowUnflattener {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detected_sequences: FxHashMap::default(),
            detected_index_vars: FxHashMap::default(),
            changed: false,
        }
    }
    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }
}

impl Default for ControlFlowUnflattener {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ControlFlowUnflattener {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            self.detect_sequence_declaration(var_decl);
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        // Only top-level dispatchers are rewritten, and in a classic script
        // their state variables are globals whose final values stay visible.
        if self.detected_sequences.is_empty() || program.source_type.is_script() {
            return;
        }

        // First pass: identify which statement indices need unflattening
        // We need to borrow program.body immutably to check while/for statements
        let mut unflatten_plan: Vec<UnflattenAction> = Vec::new();
        let mut skip_next_var_decl = false;

        for (idx, stmt) in program.body.iter().enumerate() {
            if skip_next_var_decl {
                if matches!(stmt, Statement::VariableDeclaration(_)) {
                    skip_next_var_decl = false;
                    unflatten_plan.push(UnflattenAction::Skip);
                    continue;
                }
                skip_next_var_decl = false;
            }

            match stmt {
                Statement::VariableDeclaration(var_decl) => {
                    let has_sequence = var_decl.declarations.iter().any(|d| {
                        if let BindingPattern::BindingIdentifier(ident) = &d.id {
                            self.detected_sequences.contains_key(ident.name.as_str())
                        } else {
                            false
                        }
                    });

                    if has_sequence {
                        skip_next_var_decl = true;
                        unflatten_plan.push(UnflattenAction::Skip);
                    } else {
                        unflatten_plan.push(UnflattenAction::Keep);
                    }
                }
                Statement::WhileStatement(while_stmt) if self.is_control_flow_while(while_stmt) => {
                    unflatten_plan.push(UnflattenAction::UnflattenWhile(idx));
                }
                Statement::ForStatement(for_stmt) if self.is_control_flow_for(for_stmt) => {
                    unflatten_plan.push(UnflattenAction::UnflattenFor(idx));
                }
                _ => {
                    unflatten_plan.push(UnflattenAction::Keep);
                }
            }
        }

        // Check if any unflattening is needed
        let needs_unflatten = unflatten_plan.iter().any(|a| {
            matches!(
                a,
                UnflattenAction::Skip | UnflattenAction::UnflattenWhile(_) | UnflattenAction::UnflattenFor(_)
            )
        });

        if !needs_unflatten {
            return;
        }

        // Second pass: for statements that need unflattening, extract from switch cases
        // using CloneIn (since we're reading from inside the while/for body).
        // We need to do this BEFORE draining program.body because we borrow from it.
        let mut unflattened_stmts: FxHashMap<usize, OxcVec<'a, Statement<'a>>> = FxHashMap::default();

        for action in &unflatten_plan {
            match action {
                UnflattenAction::UnflattenWhile(idx) => {
                    if let Statement::WhileStatement(while_stmt) = &program.body[*idx]
                        && let Some(stmts) = self.extract_unflattened_while(while_stmt, ctx)
                    {
                        eprintln!("[AST] Unflattened while control flow: {} statements", stmts.len());
                        unflattened_stmts.insert(*idx, stmts);
                    }
                }
                UnflattenAction::UnflattenFor(idx) => {
                    if let Statement::ForStatement(for_stmt) = &program.body[*idx]
                        && let Some(stmts) = self.extract_unflattened_for(for_stmt, ctx)
                    {
                        eprintln!("[AST] Unflattened for(;;) control flow: {} statements", stmts.len());
                        unflattened_stmts.insert(*idx, stmts);
                    }
                }
                _ => {}
            }
        }

        // Third pass: drain the body and build new_body by moving statements
        let old_body = std::mem::replace(&mut program.body, OxcVec::new_in(ctx));
        let mut new_body = OxcVec::new_in(ctx);

        for (idx, stmt) in old_body.into_iter().enumerate() {
            if idx >= unflatten_plan.len() {
                // Safety: should not happen, but move the statement anyway
                new_body.push(stmt);
                continue;
            }
            match &unflatten_plan[idx] {
                UnflattenAction::Skip => {
                    // Drop the statement (sequence var decl or index var decl)
                }
                UnflattenAction::Keep => {
                    // Move the statement directly - no clone needed
                    new_body.push(stmt);
                }
                UnflattenAction::UnflattenWhile(_) | UnflattenAction::UnflattenFor(_) => {
                    // Replace with unflattened statements
                    if let Some(stmts) = unflattened_stmts.remove(&idx) {
                        self.changed = true;
                        for s in stmts {
                            new_body.push(s);
                        }
                    } else {
                        // Unflattening failed for this one, keep original
                        new_body.push(stmt);
                    }
                }
            }
        }

        program.body = new_body;
    }
}

#[cfg(test)]
mod tests;
