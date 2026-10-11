use oxc_ast::ast::{
    ArrowFunctionExpression, BindingIdentifier, BreakStatement, ContinueStatement, Expression, Function,
    IdentifierReference, ReturnStatement, Statement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;

use super::resolver::CallResolution;
use crate::ast_deobfuscate::function_context::ContextReads;

/// How a resolved call site is replaced.
pub(super) enum Shape {
    /// The case body's statements replace the call statement.
    Splice,
    /// An arrow IIFE runs the case body, optionally binding the args parameter.
    Iife { bind_args: bool },
}

/// Names and control flow of a case body, nested functions included for names
/// and excluded for control flow.
struct BodyScan<'s> {
    scoping: &'s Scoping,
    references: Vec<(String, Option<SymbolId>)>,
    bindings: FxHashSet<SymbolId>,
    labels: Vec<String>,
    function_depth: u32,
    breakable_depth: u32,
    loop_depth: u32,
    has_return: bool,
    /// A `break`/`continue` targets a statement outside the case body.
    escapes: bool,
}

impl<'s> BodyScan<'s> {
    fn of(stmts: &[Statement<'_>], scoping: &'s Scoping) -> Self {
        let mut scan = Self {
            scoping,
            references: Vec::new(),
            bindings: FxHashSet::default(),
            labels: Vec::new(),
            function_depth: 0,
            breakable_depth: 0,
            loop_depth: 0,
            has_return: false,
            escapes: false,
        };
        for stmt in stmts {
            scan.visit_statement(stmt);
        }
        scan
    }

    fn jump(&mut self, label: Option<&str>, depth: u32) {
        if self.function_depth > 0 {
            return;
        }
        let inside = match label {
            Some(label) => self.labels.iter().any(|l| l == label),
            None => depth > 0,
        };
        self.escapes |= !inside;
    }
}

impl<'a> Visit<'a> for BodyScan<'_> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        match it {
            Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_) => {
                self.loop_depth += 1;
                self.breakable_depth += 1;
                walk::walk_statement(self, it);
                self.loop_depth -= 1;
                self.breakable_depth -= 1;
            }
            Statement::SwitchStatement(_) => {
                self.breakable_depth += 1;
                walk::walk_statement(self, it);
                self.breakable_depth -= 1;
            }
            Statement::LabeledStatement(labeled) => {
                self.labels.push(labeled.label.name.to_string());
                walk::walk_statement(self, it);
                self.labels.pop();
            }
            _ => walk::walk_statement(self, it),
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.function_depth += 1;
        walk::walk_function(self, it, flags);
        self.function_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, it);
        self.function_depth -= 1;
    }

    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        self.has_return |= self.function_depth == 0;
        walk::walk_return_statement(self, it);
    }

    fn visit_break_statement(&mut self, it: &BreakStatement<'a>) {
        self.jump(it.label.as_ref().map(|l| l.name.as_str()), self.breakable_depth);
    }

    fn visit_continue_statement(&mut self, it: &ContinueStatement<'a>) {
        self.jump(it.label.as_ref().map(|l| l.name.as_str()), self.loop_depth);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.references
            .push((it.name.to_string(), resolves_to(it, self.scoping)));
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        if let Some(symbol) = it.symbol_id.get() {
            self.bindings.insert(symbol);
        }
    }
}

fn resolves_to(ident: &IdentifierReference<'_>, scoping: &Scoping) -> Option<SymbolId> {
    scoping.get_reference(ident.reference_id()).symbol_id()
}

/// An args expression that can be dropped when the case body ignores it.
fn is_droppable(args: &Expression<'_>) -> bool {
    let Expression::ArrayExpression(array) = args else {
        return false;
    };
    array.elements.iter().all(|element| {
        element.as_expression().is_some_and(|e| {
            matches!(
                e,
                Expression::NumericLiteral(_)
                    | Expression::StringLiteral(_)
                    | Expression::BooleanLiteral(_)
                    | Expression::NullLiteral(_)
            )
        })
    })
}

/// Whether the call `call` (at `scope`) can be replaced by its case body, and
/// in which shape.
pub(super) fn plan(call: &CallResolution<'_, '_>, scope: ScopeId, scoping: &Scoping) -> Option<Shape> {
    let body = call.body;
    let dispatcher = body.dispatcher?;
    if scoping.symbol_is_mutated(dispatcher)
        || !scoping.symbol_redeclarations(dispatcher).is_empty()
        || resolves_to(call.callee, scoping) != Some(dispatcher)
        || resolves_to(call.label, scoping) != scoping.get_reference(body.label).symbol_id()
    {
        return None;
    }

    let reads = ContextReads::of_statements(&body.stmts);
    let context_kept = if call.forwards_this {
        !reads.arguments && !reads.eval
    } else {
        !reads.any()
    };
    if !context_kept {
        return None;
    }

    let scan = BodyScan::of(&body.stmts, scoping);
    if scan.escapes {
        return None;
    }
    let args_param = call.meta.args_param.as_str();
    let mut uses_args = false;
    for (name, symbol) in &scan.references {
        if symbol.is_some_and(|s| scan.bindings.contains(&s)) {
            continue;
        }
        if name == args_param {
            uses_args = true;
        } else if scoping.find_binding(scope, name.as_str().into()) != *symbol {
            return None;
        }
    }

    let bind_args = uses_args || !is_droppable(call.args);
    if scan.has_return || bind_args || !scan.bindings.is_empty() {
        Some(Shape::Iife { bind_args })
    } else {
        Some(Shape::Splice)
    }
}
