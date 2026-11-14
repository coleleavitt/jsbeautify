use oxc_allocator::{Allocator, CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, BindingPattern, BreakStatement, ContinueStatement, DoWhileStatement, Expression, ForInStatement,
    ForOfStatement, ForStatement, Function, SimpleAssignmentTarget, Statement, StaticBlock, SwitchStatement,
    UnaryOperator, UpdateOperator, VariableDeclaration, VariableDeclarationKind, VariableDeclarator, WhileStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;

use super::Ctx;

/// One dispatch loop to replace: the statements its sequence runs, and the
/// `(statement, declarator)` positions of its sequence and index variables.
pub(super) struct Plan<'a> {
    pub loop_index: usize,
    pub declarators: [(usize, usize); 2],
    pub body: OxcVec<'a, Statement<'a>>,
}

/// Plans the rewrite of `stmts[loop_index]` when it is a dispatch loop
/// `while (<truthy literal>)` or `for (;;)` whose body is exactly
/// `{ switch (S[I++]) { ... } break; }`, where
/// - `S` and `I` are declared earlier in `stmts` as `S = "a|b".split("|")` and
///   `var`/`let` `I = 0`, the discriminant is the only reference to each, and
///   neither is a global of a classic `script`;
/// - no step repeats, so each case is emitted at most once;
/// - every case test is a string literal (no `default`), no case declares
///   anything but `var`, and no case the sequence skips declares a `var`;
/// - each case the sequence reaches ends in an unlabeled `continue` (next
///   step) or `break` (leave the loop) and has no other jump to the switch or
///   the loop.
pub(super) fn plan_loop<'a>(
    stmts: &[Statement<'a>],
    loop_index: usize,
    script: bool,
    ctx: &Ctx<'a>,
) -> Option<Plan<'a>> {
    let body = match &stmts[loop_index] {
        Statement::WhileStatement(w) if is_truthy_literal(&w.test) == Some(true) => &w.body,
        Statement::ForStatement(f) if f.init.is_none() && f.test.is_none() && f.update.is_none() => &f.body,
        _ => return None,
    };
    let Statement::BlockStatement(block) = body else {
        return None;
    };
    let [Statement::SwitchStatement(switch), Statement::BreakStatement(exit)] = block.body.as_slice() else {
        return None;
    };
    if exit.label.is_some() {
        return None;
    }
    let (sequence_ref, index_ref) = dispatch_references(&switch.discriminant)?;

    let scoping = ctx.scoping();
    if scoping
        .scope_ancestors(ctx.current_scope_id())
        .any(|scope| scoping.scope_flags(scope).is_with())
    {
        return None;
    }
    let sequence_symbol = sole_reference_symbol(scoping, sequence_ref)?;
    let index_symbol = sole_reference_symbol(scoping, index_ref)?;
    // A classic script's top-level bindings are globals other scripts can read.
    if script
        && [sequence_symbol, index_symbol]
            .iter()
            .any(|&symbol| scoping.symbol_scope_id(symbol) == scoping.root_scope_id())
    {
        return None;
    }

    let earlier = &stmts[..loop_index];
    let (sequence_at, _, sequence_decl) = find_declarator(earlier, sequence_symbol)?;
    let sequence = split_sequence(sequence_decl.init.as_ref()?)?;
    let (index_at, index_kind, index_decl) = find_declarator(earlier, index_symbol)?;
    if !matches!(index_kind, VariableDeclarationKind::Var | VariableDeclarationKind::Let)
        || !matches!(&index_decl.init, Some(Expression::NumericLiteral(n)) if n.value == 0.0)
    {
        return None;
    }

    let body = run_sequence(switch, &sequence, ctx.allocator())?;
    Some(Plan {
        loop_index,
        declarators: [sequence_at, index_at],
        body,
    })
}

/// The references to `S` and `I` in a discriminant `S[I++]`.
fn dispatch_references(discriminant: &Expression<'_>) -> Option<(ReferenceId, ReferenceId)> {
    let Expression::ComputedMemberExpression(member) = discriminant else {
        return None;
    };
    let Expression::Identifier(sequence) = &member.object else {
        return None;
    };
    let Expression::UpdateExpression(update) = &member.expression else {
        return None;
    };
    if member.optional || update.prefix || update.operator != UpdateOperator::Increment {
        return None;
    }
    let SimpleAssignmentTarget::AssignmentTargetIdentifier(index) = &update.argument else {
        return None;
    };
    Some((sequence.reference_id.get()?, index.reference_id.get()?))
}

/// The symbol `reference` resolves to, when it is that symbol's only
/// reference and no direct `eval` can reach the symbol.
fn sole_reference_symbol(scoping: &Scoping, reference: ReferenceId) -> Option<SymbolId> {
    let symbol = scoping.get_reference(reference).symbol_id()?;
    let sole = scoping.get_resolved_reference_ids(symbol) == [reference]
        && scoping.symbol_redeclarations(symbol).is_empty()
        && !scoping
            .scope_flags(scoping.symbol_scope_id(symbol))
            .contains_direct_eval();
    sole.then_some(symbol)
}

fn find_declarator<'s, 'a>(
    stmts: &'s [Statement<'a>],
    symbol: SymbolId,
) -> Option<((usize, usize), VariableDeclarationKind, &'s VariableDeclarator<'a>)> {
    stmts.iter().enumerate().find_map(|(stmt_index, stmt)| {
        let Statement::VariableDeclaration(decl) = stmt else {
            return None;
        };
        decl.declarations.iter().enumerate().find_map(|(index, declarator)| {
            let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                return None;
            };
            (id.symbol_id.get() == Some(symbol)).then_some(((stmt_index, index), decl.kind, declarator))
        })
    })
}

/// The steps of `"a|b|c".split("|")` (any non-empty separator).
fn split_sequence(init: &Expression<'_>) -> Option<Vec<String>> {
    let Expression::CallExpression(call) = init else {
        return None;
    };
    let member = call.callee.as_member_expression()?;
    let Expression::StringLiteral(source) = member.object() else {
        return None;
    };
    let [Argument::StringLiteral(separator)] = call.arguments.as_slice() else {
        return None;
    };
    if call.optional || member.optional() || member.static_property_name()? != "split" || separator.value.is_empty() {
        return None;
    }
    Some(source.value.split(separator.value.as_str()).map(String::from).collect())
}

/// Whether evaluating `expr` has no side effects and gives a value of known
/// truthiness: `true`, `!0`, `!![]`, and the like.
fn is_truthy_literal(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(lit) => Some(lit.value),
        Expression::NumericLiteral(lit) => Some(lit.value != 0.0 && !lit.value.is_nan()),
        Expression::StringLiteral(lit) => Some(!lit.value.is_empty()),
        Expression::ArrayExpression(array) if array.elements.is_empty() => Some(true),
        Expression::ObjectExpression(object) if object.properties.is_empty() => Some(true),
        Expression::ParenthesizedExpression(paren) => is_truthy_literal(&paren.expression),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            is_truthy_literal(&unary.argument).map(|value| !value)
        }
        _ => None,
    }
}

/// The statements the loop runs for `sequence`: each step's case up to its
/// terminating `continue`, until a case ends in `break` or no case matches
/// (the loop's trailing `break` then leaves it).
fn run_sequence<'a>(
    switch: &SwitchStatement<'a>,
    sequence: &[String],
    allocator: &'a Allocator,
) -> Option<OxcVec<'a, Statement<'a>>> {
    let mut steps = FxHashSet::default();
    if !sequence.iter().all(|step| steps.insert(step.as_str())) {
        return None;
    }
    let mut labels = Vec::with_capacity(switch.cases.len());
    for case in &switch.cases {
        let Some(Expression::StringLiteral(test)) = &case.test else {
            return None;
        };
        if case.consequent.iter().any(declares_in_switch_scope) {
            return None;
        }
        labels.push(test.value.as_str());
    }

    let mut ran = vec![false; switch.cases.len()];
    let mut out = OxcVec::new_in(&allocator);
    for step in sequence {
        let Some(index) = labels.iter().position(|label| label == step) else {
            break;
        };
        ran[index] = true;
        let case = &switch.cases[index];
        let (last, run) = case.consequent.split_last()?;
        let next = match last {
            Statement::ContinueStatement(c) if c.label.is_none() => true,
            Statement::BreakStatement(b) if b.label.is_none() => false,
            _ => return None,
        };
        if run.iter().any(jumps_to_dispatch) {
            return None;
        }
        out.extend(run.iter().map(|stmt| stmt.clone_in_with_semantic_ids(allocator)));
        if !next {
            break;
        }
    }
    // A skipped case's `var` still binds its name in the enclosing function.
    let skipped = switch.cases.iter().zip(ran).filter(|(_, ran)| !ran);
    if skipped.flat_map(|(case, _)| &case.consequent).any(declares_var) {
        return None;
    }
    Some(out)
}

/// Whether `stmt` binds a name in the switch block's scope.
fn declares_in_switch_scope(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::VariableDeclaration(decl) => !matches!(decl.kind, VariableDeclarationKind::Var),
        _ => stmt.is_declaration(),
    }
}

/// Whether `stmt` declares a `var`, or a block-level function that sloppy
/// code also binds as a `var`, outside nested functions.
fn declares_var(stmt: &Statement<'_>) -> bool {
    let mut finder = VarFinder::default();
    finder.visit_statement(stmt);
    finder.found
}

#[derive(Default)]
struct VarFinder {
    found: bool,
}

impl<'a> Visit<'a> for VarFinder {
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.found |= it.kind == VariableDeclarationKind::Var;
        walk::walk_variable_declaration(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        self.found |= it.is_declaration();
    }

    fn visit_arrow_function_expression(&mut self, _it: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}

    fn visit_static_block(&mut self, _it: &StaticBlock<'a>) {}
}

/// Whether `stmt` has an unlabeled `break` or `continue` that targets the
/// enclosing switch or loop.
fn jumps_to_dispatch(stmt: &Statement<'_>) -> bool {
    let mut finder = JumpFinder::default();
    finder.visit_statement(stmt);
    finder.found
}

#[derive(Default)]
struct JumpFinder {
    loops: u32,
    switches: u32,
    found: bool,
}

impl JumpFinder {
    fn in_loop(&mut self, walk: impl FnOnce(&mut Self)) {
        self.loops += 1;
        walk(self);
        self.loops -= 1;
    }
}

impl<'a> Visit<'a> for JumpFinder {
    fn visit_break_statement(&mut self, it: &BreakStatement<'a>) {
        self.found |= it.label.is_none() && self.loops == 0 && self.switches == 0;
    }

    fn visit_continue_statement(&mut self, it: &ContinueStatement<'a>) {
        self.found |= it.label.is_none() && self.loops == 0;
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.switches += 1;
        walk::walk_switch_statement(self, it);
        self.switches -= 1;
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.in_loop(|v| walk::walk_while_statement(v, it));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.in_loop(|v| walk::walk_do_while_statement(v, it));
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.in_loop(|v| walk::walk_for_statement(v, it));
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.in_loop(|v| walk::walk_for_in_statement(v, it));
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.in_loop(|v| walk::walk_for_of_statement(v, it));
    }

    fn visit_function(&mut self, _it: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _it: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}

    fn visit_static_block(&mut self, _it: &StaticBlock<'a>) {}
}
