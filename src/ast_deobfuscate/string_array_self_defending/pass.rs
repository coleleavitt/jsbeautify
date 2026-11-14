use oxc_allocator::Vec as ArenaVec;
use oxc_ast::ast::{BindingPattern, Expression, Program, Statement, Str};
use oxc_span::SPAN;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::analysis::{Plan, address, analyze};

/// Inlines javascript-obfuscator's string-array decoder calls and removes the
/// rotation IIFE and decoder aliases they leave unused.
#[derive(Default)]
pub struct SelfDefendingStringArray {
    plan: Plan,
    inlined: usize,
    removed_rotations: usize,
    removed_aliases: usize,
}

impl SelfDefendingStringArray {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    #[must_use]
    pub const fn removed_rotations(&self) -> usize {
        self.removed_rotations
    }

    #[must_use]
    pub const fn changed(&self) -> bool {
        self.inlined + self.removed_rotations + self.removed_aliases > 0
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfDefendingStringArray {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.plan = analyze(program, ctx.scoping());
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let Some(value) = callee
            .reference_id
            .get()
            .and_then(|reference| self.plan.replacements.remove(&reference))
        else {
            return;
        };
        *expr = Expression::new_string_literal(SPAN, Str::from_str_in(&value, ctx), None, ctx);
        self.inlined += 1;
    }

    fn exit_statements(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, _ctx: &mut Ctx<'a>) {
        if self.plan.rotations.is_empty() && self.plan.dead_aliases.is_empty() {
            return;
        }
        let plan = &self.plan;
        let (mut removed_rotations, mut removed_aliases) = (0, 0);
        stmts.retain_mut(|stmt| match stmt {
            Statement::ExpressionStatement(expr) => {
                let is_rotation = plan.rotations.contains(&address(&**expr));
                removed_rotations += usize::from(is_rotation);
                !is_rotation
            }
            Statement::VariableDeclaration(decl) => {
                let declarators = decl.declarations.len();
                decl.declarations.retain(|declarator| {
                    !matches!(&declarator.id, BindingPattern::BindingIdentifier(id)
                        if id.symbol_id.get().is_some_and(|symbol| plan.dead_aliases.contains(&symbol)))
                });
                removed_aliases += declarators - decl.declarations.len();
                !decl.declarations.is_empty()
            }
            _ => true,
        });
        self.removed_rotations += removed_rotations;
        self.removed_aliases += removed_aliases;
    }
}
