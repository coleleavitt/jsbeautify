use oxc_allocator::{ArenaVec, CloneIn, GetAllocator};
use oxc_ast::ast::{
    ArrayExpressionElement, BindingPattern, Expression, Function, ObjectPropertyKind, Statement, UnaryOperator,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

/// Removes the declarations of dead bindings that [`super::DeadVarCollector`]
/// treats as removable: declarators of a declaration statement whose
/// initializer is side-effect free, and function declarations directly in a
/// program or function body.
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

    /// Removed variable declarators and function declarations.
    #[must_use]
    pub const fn removed_declarators(&self) -> usize {
        self.removed_declarators
    }

    fn should_remove_declarator(&self, decl: &VariableDeclarator<'_>) -> bool {
        let BindingPattern::BindingIdentifier(ident) = &decl.id else {
            return false;
        };
        ident
            .symbol_id
            .get()
            .is_some_and(|symbol| self.dead_vars.contains(&symbol))
            && decl.init.as_ref().is_none_or(is_proven_safe_initializer)
    }
}

/// The binding of a function declaration that can be removed when dead: not
/// an ambient or overload signature. The caller checks that it sits directly
/// in a program or function body.
pub(super) fn removable_function_symbol(func: &Function<'_>) -> Option<SymbolId> {
    if func.declare || func.body.is_none() {
        return None;
    }
    func.id.as_ref()?.symbol_id.get()
}

/// Only expressions whose evaluation cannot perform user code, resolve a
/// binding, access `this`, coerce a value, iterate, or throw are removable.
/// Function bodies only run when called, so function values qualify.
pub(super) fn is_proven_safe_initializer(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_) => true,
        Expression::TemplateLiteral(template) => template.expressions.is_empty(),
        // `+1n` throws, so BigInt operands are left out.
        Expression::UnaryExpression(unary) => {
            unary.operator != UnaryOperator::Delete
                && matches!(
                    unary.argument.without_parentheses(),
                    Expression::NumericLiteral(_)
                        | Expression::StringLiteral(_)
                        | Expression::BooleanLiteral(_)
                        | Expression::NullLiteral(_)
                )
        }
        Expression::ArrayExpression(array) => array.elements.iter().all(|element| match element {
            ArrayExpressionElement::SpreadElement(_) => false,
            ArrayExpressionElement::Elision(_) => true,
            _ => element.as_expression().is_some_and(is_proven_safe_initializer),
        }),
        Expression::ObjectExpression(object) => object.properties.iter().all(|property| match property {
            ObjectPropertyKind::ObjectProperty(property) => {
                !property.computed && is_proven_safe_initializer(&property.value)
            }
            ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        Expression::ParenthesizedExpression(paren) => is_proven_safe_initializer(&paren.expression),
        _ => false,
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeadVarEliminator {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt {
            let in_body = matches!(
                ctx.parent(),
                Ancestor::ProgramBody(_) | Ancestor::FunctionBodyStatements(_)
            );
            if in_body && removable_function_symbol(func).is_some_and(|symbol| self.dead_vars.contains(&symbol)) {
                if crate::ast_deobfuscate::ast_trace_enabled()
                    && let Some(id) = &func.id
                {
                    deob_trace!("[AST] Eliminating dead function: {}", id.name);
                }
                self.removed_declarators += 1;
                *stmt = Statement::new_empty_statement(SPAN, ctx);
            }
            return;
        }
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
