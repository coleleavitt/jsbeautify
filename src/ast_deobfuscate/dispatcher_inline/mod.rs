//! Dispatcher inlining pass
//!
//! Detects and inlines dispatcher object patterns:
//! ```js
//! var d = { "a": function() { return 1; }, "b": function() { return 2; } };
//! d["a"](); // Inlined to: 1
//! ```

use oxc_ast::ast::{Expression, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::{DeobfuscateState, DispatcherInfo};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod detection;
mod expression;
mod rewrite;

pub struct DispatcherInliner {
    changed: bool,
    detected_dispatchers: FxHashMap<String, DispatcherInfo>,
}

impl DispatcherInliner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            changed: false,
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

impl<'a> Traverse<'a, DeobfuscateState> for DispatcherInliner {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::VariableDeclaration(var_decl) = stmt
            && let Some(dispatcher) = self.detect_dispatcher_pattern(var_decl)
        {
            eprintln!("[AST] Found dispatcher: {}", dispatcher.var_name);
            for (key, func) in &dispatcher.functions {
                eprintln!("[AST]   - {}: {:?}", key, func.return_value);
            }
            self.detected_dispatchers
                .insert(dispatcher.var_name.clone(), dispatcher);
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if let Expression::CallExpression(call) = expr
            && let Some(new_expr) = self.try_inline_dispatcher_call(call, ctx)
        {
            *expr = new_expr;
            self.changed = true;
        }
    }
}

#[cfg(test)]
mod tests;
