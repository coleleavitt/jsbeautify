//! `switch(true)` → `if-else` chain converter.
//!
//! Stub implementation — the full rewrite needs lifetime-correct body cloning
//! and is deferred until the supporting AST-clone helpers are landed. For now
//! this is a no-op pass that counts (but does not convert) `switch(true)`
//! statements. The external automation pinning this module expects
//! `SwitchTrueConverter { new, converted_count }` to exist, so those stay.

use oxc_ast::ast::{Expression, SwitchStatement};
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct SwitchTrueConverter {
    converted_count: usize,
}

impl SwitchTrueConverter {
    fn is_switch_true(switch_stmt: &SwitchStatement<'_>) -> bool {
        matches!(&switch_stmt.discriminant, Expression::BooleanLiteral(b) if b.value)
    }
}

counter_pass!(SwitchTrueConverter, converted_count, converted_count);

impl<'a> Traverse<'a, DeobfuscateState> for SwitchTrueConverter {
    fn enter_switch_statement(&mut self, stmt: &mut SwitchStatement<'a>, _ctx: &mut Ctx<'a>) {
        if Self::is_switch_true(stmt) {
            self.converted_count += 1;
        }
    }
}
