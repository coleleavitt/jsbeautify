use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{BindingPattern, Expression, Program, Statement, VariableDeclarator};
use oxc_ast_visit::{Visit, walk};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

pub struct DeadVarEliminator {
    dead_vars: FxHashSet<SymbolId>,
    removed_declarators: usize,
}

impl DeadVarEliminator {
    #[must_use]
    pub const fn new(dead_vars: FxHashSet<SymbolId>) -> Self {
        Self {
            dead_vars,
            removed_declarators: 0,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.removed_declarators > 0
    }

    #[must_use]
    pub const fn removed_declarators(&self) -> usize {
        self.removed_declarators
    }

    fn should_remove_declarator(&self, decl: &VariableDeclarator<'_>) -> bool {
        is_removable(decl, &self.dead_vars)
    }
}

/// Whether [`DeadVarEliminator`] would remove a declarator of `program`:
/// the same decision, made without changing the program.
pub(super) fn removes_any(program: &Program<'_>, dead_vars: &FxHashSet<SymbolId>) -> bool {
    struct Probe<'d> {
        dead_vars: &'d FxHashSet<SymbolId>,
        found: bool,
    }
    impl<'a> Visit<'a> for Probe<'_> {
        fn visit_statement(&mut self, stmt: &Statement<'a>) {
            if self.found {
                return;
            }
            if let Statement::VariableDeclaration(var_decl) = stmt
                && var_decl
                    .declarations
                    .iter()
                    .any(|decl| is_removable(decl, self.dead_vars))
            {
                self.found = true;
                return;
            }
            walk::walk_statement(self, stmt);
        }
    }
    let mut probe = Probe {
        dead_vars,
        found: false,
    };
    probe.visit_program(program);
    probe.found
}

fn is_removable(decl: &VariableDeclarator<'_>, dead_vars: &FxHashSet<SymbolId>) -> bool {
    let BindingPattern::BindingIdentifier(ident) = &decl.id else {
        return false;
    };
    let Some(symbol_id) = ident.symbol_id.get() else {
        return false;
    };
    if !dead_vars.contains(&symbol_id) {
        return false;
    }
    decl.init.as_ref().is_none_or(is_proven_safe_initializer)
}

/// Only expressions whose evaluation cannot perform user code, resolve a
/// binding, access `this`, coerce a value, iterate, or throw are removable.
fn is_proven_safe_initializer(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_) => true,
        Expression::ParenthesizedExpression(paren) => is_proven_safe_initializer(&paren.expression),
        _ => false,
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeadVarEliminator {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            let all_dead = var_decl.declarations.iter().all(|d| self.should_remove_declarator(d));

            if all_dead && !var_decl.declarations.is_empty() {
                for decl in &var_decl.declarations {
                    if let BindingPattern::BindingIdentifier(ident) = &decl.id
                        && crate::ast_deobfuscate::ast_trace_enabled()
                    {
                        deob_trace!("[AST] Eliminating dead variable: {}", ident.name);
                    }
                }
                self.removed_declarators += var_decl.declarations.len();
                *stmt = Statement::new_empty_statement(SPAN, ctx);
                return;
            }

            let has_dead = var_decl.declarations.iter().any(|d| self.should_remove_declarator(d));
            if has_dead {
                let mut new_declarations = ArenaVec::new_in(ctx);
                for decl in &var_decl.declarations {
                    if !self.should_remove_declarator(decl) {
                        new_declarations.push(Self::clone_declarator(decl, ctx));
                    } else if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                        if crate::ast_deobfuscate::ast_trace_enabled() {
                            deob_trace!("[AST] Eliminating dead variable: {}", ident.name);
                        }
                        self.removed_declarators += 1;
                    }
                }

                if new_declarations.is_empty() {
                    *stmt = Statement::new_empty_statement(SPAN, ctx);
                } else {
                    *stmt = Statement::new_variable_declaration(
                        SPAN,
                        var_decl.kind,
                        new_declarations,
                        var_decl.declare,
                        ctx,
                    );
                }
            }
        }
    }
}

impl DeadVarEliminator {
    fn clone_declarator<'a>(decl: &VariableDeclarator<'a>, ctx: &mut Ctx<'a>) -> VariableDeclarator<'a> {
        decl.clone_in_with_semantic_ids(ctx.allocator())
    }
}
