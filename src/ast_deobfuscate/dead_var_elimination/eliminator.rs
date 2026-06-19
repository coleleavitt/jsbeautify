use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    ArrayExpressionElement, BindingPattern, EmptyStatement, Expression, ObjectPropertyKind, Statement, UnaryOperator,
    VariableDeclaration, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashSet;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

pub struct DeadVarEliminator {
    dead_vars: FxHashSet<String>,
    changed: bool,
}

impl DeadVarEliminator {
    #[must_use]
    pub const fn new(dead_vars: FxHashSet<String>) -> Self {
        Self {
            dead_vars,
            changed: false,
        }
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn should_remove_declarator(&self, decl: &VariableDeclarator<'_>) -> bool {
        if let BindingPattern::BindingIdentifier(ident) = &decl.id {
            let name = ident.name.as_str();
            if self.dead_vars.contains(name) {
                if let Some(init) = &decl.init
                    && Self::has_side_effects(init)
                {
                    return false;
                }
                return true;
            }
        }
        false
    }

    fn has_side_effects(expr: &Expression<'_>) -> bool {
        match expr {
            Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::Identifier(_)
            | Expression::ThisExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_) => false,

            Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
                ArrayExpressionElement::SpreadElement(spread) => Self::has_side_effects(&spread.argument),
                ArrayExpressionElement::Elision(_) => false,
                _ => el.as_expression().is_none_or(Self::has_side_effects),
            }),

            Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    let key_has_side_effects = if p.computed {
                        p.key.as_expression().is_some_and(Self::has_side_effects)
                    } else {
                        false
                    };
                    Self::has_side_effects(&p.value) || key_has_side_effects
                }
                ObjectPropertyKind::SpreadProperty(spread) => Self::has_side_effects(&spread.argument),
            }),

            Expression::UnaryExpression(unary) => {
                matches!(unary.operator, UnaryOperator::Delete) || Self::has_side_effects(&unary.argument)
            }

            Expression::BinaryExpression(binary) => {
                Self::has_side_effects(&binary.left) || Self::has_side_effects(&binary.right)
            }

            Expression::LogicalExpression(logical) => {
                Self::has_side_effects(&logical.left) || Self::has_side_effects(&logical.right)
            }

            Expression::ConditionalExpression(cond) => {
                Self::has_side_effects(&cond.test)
                    || Self::has_side_effects(&cond.consequent)
                    || Self::has_side_effects(&cond.alternate)
            }

            Expression::SequenceExpression(seq) => seq.expressions.iter().any(|e| Self::has_side_effects(e)),

            Expression::ParenthesizedExpression(paren) => Self::has_side_effects(&paren.expression),

            _ => true,
        }
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
                        eprintln!("[AST] Eliminating dead variable: {}", ident.name);
                    }
                }
                self.changed = true;
                *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                }));
                return;
            }

            let has_dead = var_decl.declarations.iter().any(|d| self.should_remove_declarator(d));
            if has_dead {
                let mut new_declarations = ctx.ast.vec();
                for decl in &var_decl.declarations {
                    if !self.should_remove_declarator(decl) {
                        new_declarations.push(Self::clone_declarator(decl, ctx));
                    } else if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                        if crate::ast_deobfuscate::ast_trace_enabled() {
                            eprintln!("[AST] Eliminating dead variable: {}", ident.name);
                        }
                        self.changed = true;
                    }
                }

                if new_declarations.is_empty() {
                    *stmt = Statement::EmptyStatement(ctx.ast.alloc(EmptyStatement {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                    }));
                } else {
                    *stmt = Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        kind: var_decl.kind,
                        declarations: new_declarations,
                        declare: var_decl.declare,
                    }));
                }
            }
        }
    }
}

impl DeadVarEliminator {
    fn clone_declarator<'a>(decl: &VariableDeclarator<'a>, ctx: &mut Ctx<'a>) -> VariableDeclarator<'a> {
        decl.clone_in_with_semantic_ids(ctx.ast.allocator)
    }
}
