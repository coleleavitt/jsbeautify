use oxc_allocator::CloneIn;
use oxc_ast::ast::{BindingPattern, EmptyStatement, Expression, IdentifierReference, Program, Statement};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;
use std::cmp::Reverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
mod stats;

use stats::{TrackerStats, collect_tracker_stats};

/// Finds a single variable `Ot` that:
///   - is declared exactly once at top-level (or inside the BMP IIFE body)
///   - is only ever written via `.push(x)`, `.pop()`, `.splice(..., Infinity, x)`
///   - is never read (no `Ot[i]`, no `Ot.length`, no `Ot === ...` etc.)
///
/// …and removes every `Ot.push/pop/splice` expression statement plus the
/// declaration. Also handles the common comma-operator usage
/// `return Ot.pop(), foo = bar, foo;` by rewriting to `return foo = bar, foo;`.
pub struct StackTrackerRemover {
    tracker_names: FxHashSet<String>,
    removed_calls: usize,
}

impl StackTrackerRemover {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracker_names: FxHashSet::default(),
            removed_calls: 0,
        }
    }

    #[must_use]
    pub const fn removed_call_count(&self) -> usize {
        self.removed_calls
    }

    #[must_use]
    pub const fn tracker_names(&self) -> &FxHashSet<String> {
        &self.tracker_names
    }

    /// Scan program to identify tracker variables. Must be called before traversal.
    pub fn detect(&mut self, program: &Program<'_>) {
        let mut candidates = FxHashMap::<String, TrackerStats>::default();
        collect_tracker_stats(&program.body, &mut candidates);

        let mut ranked: Vec<(&String, &TrackerStats)> = candidates.iter().collect();
        ranked.sort_by_key(|(_, stats)| Reverse(stats.write_calls));

        eprintln!(
            "[AKAMAI/stack-tracker] Scanned {} candidate identifiers. Top 10 by write_calls:",
            candidates.len()
        );
        for (name, stats) in ranked.iter().take(10) {
            let verdict = if stats.is_pure_tracker() {
                "TRACKER"
            } else if !stats.declared {
                "(no init-less decl)"
            } else if stats.reads > 0 {
                "(has reads)"
            } else if stats.write_calls < 4 {
                "(too few writes)"
            } else {
                "(unknown)"
            };
            eprintln!(
                "[AKAMAI/stack-tracker]   {:>6} : writes={:>4}  reads={:>4}  declared={}  => {}",
                name, stats.write_calls, stats.reads, stats.declared, verdict
            );
        }

        for (name, stats) in candidates {
            if stats.is_pure_tracker() {
                eprintln!(
                    "[AKAMAI/stack-tracker] PROMOTING '{}' to tracker ({} writes, {} reads)",
                    name, stats.write_calls, stats.reads
                );
                self.tracker_names.insert(name);
            }
        }

        if self.tracker_names.is_empty() {
            eprintln!("[AKAMAI/stack-tracker] No pure stack-tracker variable identified");
        } else {
            eprintln!("[AKAMAI/stack-tracker] Will strip calls on: {:?}", self.tracker_names);
        }
    }

    fn is_tracker_call(&self, expr: &Expression<'_>) -> bool {
        let Expression::CallExpression(call) = expr else {
            return false;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return false;
        };
        let Expression::Identifier(id) = &sme.object else {
            return false;
        };
        if !self.tracker_names.contains(id.name.as_str()) {
            return false;
        }
        matches!(sme.property.name.as_str(), "push" | "pop" | "splice")
    }
}
impl Default for StackTrackerRemover {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for StackTrackerRemover {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        // 1. Drop `var Ot;` declarations where Ot is a known tracker.
        if let Statement::VariableDeclaration(decl) = stmt {
            let all_trackers = decl.declarations.iter().all(|d| {
                if let BindingPattern::BindingIdentifier(id) = &d.id {
                    self.tracker_names.contains(id.name.as_str()) && d.init.is_none()
                } else {
                    false
                }
            });
            if all_trackers && !decl.declarations.is_empty() {
                let names: Vec<&str> = decl
                    .declarations
                    .iter()
                    .filter_map(|d| match &d.id {
                        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
                        _ => None,
                    })
                    .collect();
                eprintln!("[AKAMAI/stack-tracker] removing declaration: var {};", names.join(", "));
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
                return;
            }
        }

        // 2. Drop `Ot.push(x);` / `Ot.pop();` / `Ot.splice(...);` expression statements.
        if let Statement::ExpressionStatement(es) = stmt
            && self.is_tracker_call(&es.expression)
        {
            if self.removed_calls < 10 {
                eprintln!("[AKAMAI/stack-tracker] removing call #{}", self.removed_calls + 1);
            } else if self.removed_calls == 10 {
                eprintln!("[AKAMAI/stack-tracker] ... further removals silenced");
            }
            self.removed_calls += 1;
            *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            }));
        }
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        // Handle sequence expressions: `return Ot.pop(), x = y, x`
        // Drop tracker calls from comma sequences.
        if let Expression::SequenceExpression(seq) = expr {
            let before = seq.expressions.len();
            let mut kept = ctx.ast.vec();
            for e in &seq.expressions {
                if self.is_tracker_call(e) {
                    self.removed_calls += 1;
                    continue;
                }
                kept.push(e.clone_in_with_semantic_ids(ctx.ast.allocator));
            }
            if kept.len() == before {
                return;
            }
            if kept.is_empty() {
                // Shouldn't happen, but be safe.
                *expr = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident("undefined"),
                    reference_id: Cell::default(),
                }));
                return;
            }
            if kept.len() == 1 {
                // Collapse single-element sequence into plain expression.
                let mut iter = kept.into_iter();
                if let Some(only) = iter.next() {
                    *expr = only;
                }
                return;
            }
            seq.expressions = kept;
        }
    }
}
