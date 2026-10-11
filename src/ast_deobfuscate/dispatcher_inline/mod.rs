//! Dispatcher inlining pass
//!
//! Detects and inlines dispatcher object patterns:
//! ```js
//! var d = { "a": function() { return 1; }, "b": function() { return 2; } };
//! d["a"](); // Inlined to: 1
//! ```
//!
//! `d` is matched by `SymbolId` and must never be reassigned or used other than
//! as the object of a member call, so no code can replace one of its functions.
//! Calls with non-literal arguments are kept, since inlining drops them.

use oxc_ast::ast::{Argument, CallExpression, Expression, Program, Statement};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::{DeobfuscateState, DispatcherInfo};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod detection;
mod expression;
mod rewrite;

pub struct DispatcherInliner {
    changed: bool,
    /// Bindings whose every reference is the object of a member call.
    call_only: FxHashSet<SymbolId>,
    detected_dispatchers: FxHashMap<SymbolId, DispatcherInfo>,
}

impl DispatcherInliner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            changed: false,
            call_only: FxHashSet::default(),
            detected_dispatchers: FxHashMap::default(),
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }
}

impl Default for DispatcherInliner {
    fn default() -> Self {
        Self::new()
    }
}

/// How many references of each symbol are the object of `x.k(...)`/`x["k"](...)`.
struct MemberCallObjects<'s> {
    scoping: &'s oxc_semantic::Scoping,
    counts: FxHashMap<SymbolId, usize>,
}

impl<'a> Visit<'a> for MemberCallObjects<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Some(member) = it.callee.as_member_expression()
            && let Expression::Identifier(object) = member.object()
            && let Some(symbol) = self.scoping.get_reference(object.reference_id()).symbol_id()
        {
            *self.counts.entry(symbol).or_default() += 1;
        }
        walk::walk_call_expression(self, it);
    }
}

fn literal_arguments(arguments: &[Argument<'_>]) -> bool {
    arguments.iter().all(|arg| {
        matches!(
            arg,
            Argument::NumericLiteral(_)
                | Argument::StringLiteral(_)
                | Argument::BooleanLiteral(_)
                | Argument::NullLiteral(_)
        )
    })
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatcherInliner {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let scoping = ctx.scoping();
        let mut objects = MemberCallObjects {
            scoping,
            counts: FxHashMap::default(),
        };
        objects.visit_program(program);
        self.call_only = objects
            .counts
            .into_iter()
            .filter(|&(symbol, count)| {
                !scoping.symbol_is_mutated(symbol)
                    && scoping.symbol_redeclarations(symbol).is_empty()
                    && !is_global_binding(program.source_type, scoping, symbol)
                    && scoping.get_resolved_reference_ids(symbol).len() == count
            })
            .map(|(symbol, _)| symbol)
            .collect();
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::VariableDeclaration(var_decl) = stmt
            && let Some(dispatcher) = self.detect_dispatcher_pattern(var_decl)
            && let Some(symbol) = dispatcher.symbol
            && self.call_only.contains(&symbol)
        {
            eprintln!("[AST] Found dispatcher: {}", dispatcher.var_name);
            for (key, func) in &dispatcher.functions {
                eprintln!("[AST]   - {}: {:?}", key, func.return_value);
            }
            self.detected_dispatchers.insert(symbol, dispatcher);
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && literal_arguments(&call.arguments)
            && let Some(new_expr) = self.try_inline_dispatcher_call(call, ctx)
        {
            *expr = new_expr;
            self.changed = true;
        }
    }
}

#[cfg(test)]
mod tests;
