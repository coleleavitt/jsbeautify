use oxc_allocator::{ArenaVec, TakeIn};
use oxc_ast::ast::{Expression, Program, Statement};
use oxc_semantic::Scoping;
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
mod stats;

use stats::{Trackers, find_trackers};

/// Finds variables like `Ot` that:
///   - are declared once by `var Ot;` (not at the top level of a classic script)
///   - are reset with `Ot = [...]` and otherwise used only by `Ot.push(x)`,
///     `Ot.pop()` and `Ot.splice(...)` calls with side-effect-free arguments
///   - are never read: every such operation's value is discarded
///
/// …and removes every such operation plus the declaration. Also handles the
/// common comma-operator usage `return Ot.pop(), foo = bar, foo;` by rewriting
/// to `return foo = bar, foo;`.
///
/// Trackers and their uses are matched by `SymbolId` and `ReferenceId`, so a
/// shadowing binding of the same name is left alone.
pub struct StackTrackerRemover {
    trackers: Trackers,
    tracker_names: Vec<String>,
    removed_calls: usize,
}

impl StackTrackerRemover {
    #[must_use]
    pub fn new() -> Self {
        Self {
            trackers: Trackers::default(),
            tracker_names: Vec::new(),
            removed_calls: 0,
        }
    }

    #[must_use]
    pub const fn removed_call_count(&self) -> usize {
        self.removed_calls
    }

    #[must_use]
    pub fn tracker_names(&self) -> &[String] {
        &self.tracker_names
    }

    /// Scan program to identify tracker variables. Must be called before
    /// traversal, with the scoping the traversal uses.
    pub fn detect(&mut self, program: &Program<'_>, scoping: &Scoping) {
        self.trackers = find_trackers(program, scoping);
        self.tracker_names = self
            .trackers
            .symbols
            .iter()
            .map(|&symbol| scoping.symbol_name(symbol).to_string())
            .collect();
        self.tracker_names.sort_unstable();
        if self.tracker_names.is_empty() {
            deob_trace!("[AKAMAI/stack-tracker] No pure stack-tracker variable identified");
        } else {
            deob_trace!("[AKAMAI/stack-tracker] Will strip calls on: {:?}", self.tracker_names);
        }
    }
}

impl Default for StackTrackerRemover {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for StackTrackerRemover {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let remove = match stmt {
            // 1. `var Ot;` where every declared binding is a tracker.
            Statement::VariableDeclaration(decl) => decl.declarations.iter().all(|d| {
                d.id.get_binding_identifier()
                    .and_then(|id| id.symbol_id.get())
                    .is_some_and(|symbol| self.trackers.symbols.contains(&symbol))
            }),
            // 2. `Ot.push(x);`, `Ot.pop();`, `Ot = [];`, or a sequence of them.
            Statement::ExpressionStatement(es) => match &es.expression {
                Expression::SequenceExpression(seq) => seq.expressions.iter().all(|e| self.trackers.is_removable(e)),
                expr => self.trackers.is_removable(expr),
            },
            _ => false,
        };
        if !remove {
            return;
        }
        if let Statement::ExpressionStatement(es) = stmt {
            self.removed_calls += match &es.expression {
                Expression::SequenceExpression(seq) => seq.expressions.len(),
                _ => 1,
            };
        }
        *stmt = Statement::new_empty_statement(SPAN, &ctx.ast);
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        // `return Ot.pop(), x = y, x`: drop tracker operations whose value the
        // sequence discards. A statement's whole sequence goes in `exit_statement`.
        let Expression::SequenceExpression(seq) = expr else {
            return;
        };
        let statement = matches!(ctx.parent(), Ancestor::ExpressionStatementExpression(_));
        let Some(last) = seq.expressions.len().checked_sub(1) else {
            return;
        };
        let removable = |(i, e): (usize, &Expression<'a>)| (i < last || statement) && self.trackers.is_removable(e);
        let count = seq
            .expressions
            .iter()
            .enumerate()
            .filter(|&item| removable(item))
            .count();
        if count == 0 || count == seq.expressions.len() {
            return;
        }
        let mut kept = ArenaVec::with_capacity_in(seq.expressions.len() - count, &ctx.ast);
        for (i, e) in seq.expressions.iter_mut().enumerate() {
            if !removable((i, e)) {
                kept.push(e.take_in(&ctx.ast));
            }
        }
        self.removed_calls += count;
        if kept.len() == 1
            && let Some(only) = kept.pop()
        {
            *expr = only;
        } else {
            seq.expressions = kept;
        }
    }
}
