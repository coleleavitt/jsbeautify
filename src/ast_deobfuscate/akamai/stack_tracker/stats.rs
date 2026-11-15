use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, Declaration, ExportDeclaration, Expression,
    ExpressionStatement, IdentifierReference, Program, SequenceExpression, UnaryOperator, VariableDeclaration,
    VariableDeclarationKind,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::{SymbolFlags, SymbolId};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::with_scope::{redirectable_symbols, scopes_in_with_bodies};

use super::super::only_local_access;

/// Fewest `.push` / `.pop` / `.splice` calls that make a binding a tracker.
const MIN_TRACKER_CALLS: usize = 4;

/// A use of a tracker binding that can be deleted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `S.push(…)`, `S.pop()` or `S.splice(…)`.
    Call,
    /// `S = […]`.
    Reset,
}

/// The binding `expr` operates on when it has a tracker operation's shape.
fn op<'b, 'a>(expr: &'b Expression<'a>) -> Option<(&'b IdentifierReference<'a>, Op)> {
    match expr {
        Expression::CallExpression(call) if !call.optional => {
            let Expression::StaticMemberExpression(member) = &call.callee else {
                return None;
            };
            let Expression::Identifier(target) = &member.object else {
                return None;
            };
            (!member.optional && matches!(member.property.name.as_str(), "push" | "pop" | "splice"))
                .then_some((target, Op::Call))
        }
        Expression::AssignmentExpression(assign) if assign.operator == AssignmentOperator::Assign => {
            match (&assign.left, &assign.right) {
                (AssignmentTarget::AssignmentTargetIdentifier(target), Expression::ArrayExpression(_)) => {
                    Some((target, Op::Reset))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Every use of a tracker binding, keyed by reference: the tracker
/// operations whose value is discarded and whose arguments have no side
/// effects. A binding is a tracker when every reference to it is one of
/// these, so deleting them all, and its `var`, is unobservable.
#[derive(Default)]
pub(super) struct Trackers {
    pub(super) symbols: FxHashSet<SymbolId>,
    pub(super) removable: FxHashSet<ReferenceId>,
}

impl Trackers {
    /// Whether `expr` is one of the tracker operations to delete.
    pub(super) fn is_removable(&self, expr: &Expression<'_>) -> bool {
        op(expr).is_some_and(|(target, _)| self.removable.contains(&target.reference_id()))
    }
}

pub(super) fn find_trackers(program: &Program<'_>, scoping: &Scoping) -> Trackers {
    let mut uses = Uses {
        scoping,
        with_scopes: scopes_in_with_bodies(scoping),
        declared: FxHashSet::default(),
        removable: FxHashMap::default(),
    };
    uses.visit_program(program);
    let Uses {
        declared, removable, ..
    } = uses;
    let redirectable = redirectable_symbols(program, scoping);
    let symbols: FxHashSet<SymbolId> = declared
        .into_iter()
        .filter(|&symbol| {
            let references = scoping.get_resolved_reference_ids(symbol);
            let ops = || references.iter().filter_map(|id| removable.get(id));
            references.iter().all(|id| removable.contains_key(id))
                && ops().filter(|&&op| op == Op::Call).count() >= MIN_TRACKER_CALLS
                && ops().any(|&op| op == Op::Reset)
                && only_local_access(program, scoping, &redirectable, symbol)
        })
        .collect();
    let removable = symbols
        .iter()
        .flat_map(|&symbol| scoping.get_resolved_reference_ids(symbol).iter().copied())
        .collect();
    Trackers { symbols, removable }
}

struct Uses<'s> {
    scoping: &'s Scoping,
    with_scopes: FxHashSet<ScopeId>,
    /// Bindings of `var S;` declarators.
    declared: FxHashSet<SymbolId>,
    removable: FxHashMap<ReferenceId, Op>,
}

impl Uses<'_> {
    /// Records `expr`, whose value is discarded, when it is a tracker
    /// operation without side effects besides the one on its target.
    fn discarded(&mut self, expr: &Expression<'_>) {
        let Some((target, op)) = op(expr) else {
            return;
        };
        let pure = match expr {
            Expression::CallExpression(call) => call.arguments.iter().all(|arg| match arg {
                Argument::SpreadElement(_) => false,
                arg => self.is_pure(arg.to_expression()),
            }),
            Expression::AssignmentExpression(assign) => match &assign.right {
                Expression::ArrayExpression(array) => array.elements.iter().all(|element| match element {
                    ArrayExpressionElement::SpreadElement(_) => false,
                    ArrayExpressionElement::Elision(_) => true,
                    element => self.is_pure(element.to_expression()),
                }),
                _ => false,
            },
            _ => false,
        };
        if pure {
            self.removable.insert(target.reference_id(), op);
        }
    }

    /// Evaluating `expr` can neither throw nor run code: a literal, or a read
    /// of a binding without a temporal dead zone that no `with` object hides.
    fn is_pure(&self, expr: &Expression<'_>) -> bool {
        match expr {
            Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_) => true,
            Expression::UnaryExpression(unary) => {
                matches!(unary.operator, UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus)
                    && matches!(unary.argument, Expression::NumericLiteral(_))
            }
            Expression::Identifier(ident) => {
                let reference = self.scoping.get_reference(ident.reference_id());
                !self.with_scopes.contains(&reference.scope_id())
                    && reference.symbol_id().is_some_and(|symbol| {
                        self.scoping
                            .symbol_flags(symbol)
                            .intersects(SymbolFlags::FunctionScopedVariable | SymbolFlags::Function)
                    })
            }
            Expression::ParenthesizedExpression(paren) => self.is_pure(&paren.expression),
            _ => false,
        }
    }
}

impl<'a> Visit<'a> for Uses<'_> {
    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        if decl.kind == VariableDeclarationKind::Var {
            for declarator in &decl.declarations {
                if declarator.init.is_none()
                    && let Some(id) = declarator.id.get_binding_identifier()
                    && let Some(symbol) = id.symbol_id.get()
                {
                    self.declared.insert(symbol);
                }
            }
        }
        walk::walk_variable_declaration(self, decl);
    }

    fn visit_export_declaration(&mut self, export: &ExportDeclaration<'a>) {
        // Importers read an exported `var`, so it is not recorded as declared.
        if let Declaration::VariableDeclaration(var) = &export.declaration {
            walk::walk_variable_declaration(self, var);
        } else {
            walk::walk_export_declaration(self, export);
        }
    }

    fn visit_expression_statement(&mut self, stmt: &ExpressionStatement<'a>) {
        if let Expression::SequenceExpression(seq) = &stmt.expression {
            for expr in &seq.expressions {
                self.discarded(expr);
                self.visit_expression(expr);
            }
        } else {
            self.discarded(&stmt.expression);
            walk::walk_expression_statement(self, stmt);
        }
    }

    fn visit_sequence_expression(&mut self, seq: &SequenceExpression<'a>) {
        if let Some((_, init)) = seq.expressions.split_last() {
            for expr in init {
                self.discarded(expr);
            }
        }
        walk::walk_sequence_expression(self, seq);
    }
}
