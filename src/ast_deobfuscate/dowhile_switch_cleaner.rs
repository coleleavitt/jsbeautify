//! Dead-case pruner for do-while-switch dispatchers.
//!
//! Consumes a [`DoWhileDispatcherMap`] (from [`super::dowhile_switch_detector`])
//! and removes switch cases that are unreachable from any entry point.
//!
//! Reachability is computed by:
//! 1. Scanning call sites to find entry states (e.g. `Ql(Q4, [...])`)
//! 2. Following state transitions from each reachable case to its successors
//! 3. Iterating until fixpoint
//! 4. Removing cases whose labels are not in the reachable set

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use super::dowhile_switch_detector::{DoWhileDispatcherMap, StateTransition};
use super::state::DeobfuscateState;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct DoWhileSwitchCleaner {
    dispatchers: DoWhileDispatcherMap,
    reachable_per_dispatcher: FxHashMap<String, FxHashSet<String>>,
    pruned_cases: usize,
}

impl DoWhileSwitchCleaner {
    #[must_use]
    pub fn new(dispatchers: DoWhileDispatcherMap, program: &Program<'_>) -> Self {
        let entry_states = collect_entry_states(program, &dispatchers);
        let reachable_per_dispatcher = compute_reachability(&dispatchers, &entry_states);
        Self {
            dispatchers,
            reachable_per_dispatcher,
            pruned_cases: 0,
        }
    }

    #[must_use]
    pub const fn pruned_cases(&self) -> usize {
        self.pruned_cases
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DoWhileSwitchCleaner {
    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                let Some(id) = &func.id else { return };
                let name = id.name.as_str();
                self.try_prune_dispatcher(name, func, ctx);
            }
            Statement::VariableDeclaration(decl) => {
                for d in &mut decl.declarations {
                    let BindingPattern::BindingIdentifier(var_id) = &d.id else {
                        continue;
                    };
                    let var_name = var_id.name.as_str().to_string();
                    let Some(Expression::FunctionExpression(func)) = &mut d.init else {
                        continue;
                    };
                    self.try_prune_dispatcher_mut(&var_name, func.as_mut(), ctx);
                }
            }
            _ => {}
        }
    }
}

impl DoWhileSwitchCleaner {
    fn try_prune_dispatcher<'a>(&mut self, name: &str, func: &Function<'a>, _ctx: &mut Ctx<'a>) {
        let Some(reachable) = self.reachable_per_dispatcher.get(name) else {
            return;
        };
        if !self.dispatchers.contains_key(name) {
            return;
        }
        let Some(body) = &func.body else { return };

        let switch = body.statements.iter().find_map(|s| {
            if let Statement::DoWhileStatement(dw) = s {
                extract_switch_from_body(&dw.body)
            } else {
                None
            }
        });
        let Some(switch) = switch else { return };

        let total = switch.cases.len();
        let dead_count = switch
            .cases
            .iter()
            .filter(|c| {
                c.test
                    .as_ref()
                    .and_then(|t| {
                        if let Expression::Identifier(id) = t {
                            Some(id.name.as_str())
                        } else {
                            None
                        }
                    })
                    .is_some_and(|label| !reachable.contains(label))
            })
            .count();

        if dead_count > 0 {
            eprintln!(
                "[DOWHILE] dispatcher {name}: {dead_count}/{total} cases are dead (not pruning immutable FunctionDeclaration)"
            );
        }
    }

    fn try_prune_dispatcher_mut<'a>(&mut self, name: &str, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        let Some(reachable) = self.reachable_per_dispatcher.get(name) else {
            return;
        };
        if !self.dispatchers.contains_key(name) {
            return;
        }
        let Some(body) = &mut func.body else { return };

        let switch = body.statements.iter_mut().find_map(|s| {
            if let Statement::DoWhileStatement(dw) = s {
                extract_switch_from_body_mut(&mut dw.body)
            } else {
                None
            }
        });
        let Some(switch) = switch else { return };

        let total = switch.cases.len();
        let before = switch.cases.len();

        switch.cases.retain(|case| {
            let Some(test) = &case.test else {
                return true; // keep default case
            };
            let Expression::Identifier(id) = test else {
                return true; // keep non-identifier cases
            };
            reachable.contains(id.name.as_str())
        });

        let removed = before - switch.cases.len();
        if removed > 0 {
            eprintln!("[DOWHILE] pruned {removed}/{total} dead cases from {name}");
            self.pruned_cases += removed;
        }
    }
}

fn extract_switch_from_body<'a, 'b>(body: &'b Statement<'a>) -> Option<&'b oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(block) => block.body.iter().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                Some(sw.as_ref())
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn extract_switch_from_body_mut<'a, 'b>(
    body: &'b mut Statement<'a>,
) -> Option<&'b mut oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(block) => block.body.iter_mut().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                Some(sw.as_mut())
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn collect_entry_states(
    program: &Program<'_>,
    dispatchers: &DoWhileDispatcherMap,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut entries: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    scan_stmts_for_entries(&program.body, dispatchers, &mut entries);
    entries
}

fn scan_stmts_for_entries<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    for stmt in stmts {
        scan_stmt_for_entries(stmt, dispatchers, entries);
    }
}

fn scan_stmt_for_entries<'a>(
    stmt: &Statement<'a>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    match stmt {
        Statement::ExpressionStatement(es) => scan_expr_for_entries(&es.expression, dispatchers, entries),
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let Some(init) = &d.init {
                    scan_expr_for_entries(init, dispatchers, entries);
                }
            }
        }
        Statement::ReturnStatement(r) => {
            if let Some(arg) = &r.argument {
                scan_expr_for_entries(arg, dispatchers, entries);
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                scan_stmts_for_entries(&body.statements, dispatchers, entries);
            }
        }
        Statement::BlockStatement(b) => scan_stmts_for_entries(&b.body, dispatchers, entries),
        Statement::IfStatement(ifs) => {
            scan_expr_for_entries(&ifs.test, dispatchers, entries);
            scan_stmt_for_entries(&ifs.consequent, dispatchers, entries);
            if let Some(alt) = &ifs.alternate {
                scan_stmt_for_entries(alt, dispatchers, entries);
            }
        }
        Statement::ForStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::ForInStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::ForOfStatement(f) => scan_stmt_for_entries(&f.body, dispatchers, entries),
        Statement::WhileStatement(w) => scan_stmt_for_entries(&w.body, dispatchers, entries),
        Statement::DoWhileStatement(d) => scan_stmt_for_entries(&d.body, dispatchers, entries),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                scan_stmts_for_entries(&case.consequent, dispatchers, entries);
            }
        }
        Statement::TryStatement(t) => {
            scan_stmts_for_entries(&t.block.body, dispatchers, entries);
            if let Some(h) = &t.handler {
                scan_stmts_for_entries(&h.body.body, dispatchers, entries);
            }
            if let Some(f) = &t.finalizer {
                scan_stmts_for_entries(&f.body, dispatchers, entries);
            }
        }
        Statement::LabeledStatement(l) => scan_stmt_for_entries(&l.body, dispatchers, entries),
        _ => {}
    }
}

fn scan_expr_for_entries<'a>(
    expr: &Expression<'a>,
    dispatchers: &DoWhileDispatcherMap,
    entries: &mut FxHashMap<String, FxHashSet<String>>,
) {
    match expr {
        Expression::CallExpression(call) => {
            if let Some((disp_name, state_label)) = extract_dispatcher_call(call, dispatchers) {
                entries.entry(disp_name).or_default().insert(state_label);
            }
            scan_expr_for_entries(&call.callee, dispatchers, entries);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                scan_stmts_for_entries(&body.statements, dispatchers, entries);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            scan_stmts_for_entries(&func.body.statements, dispatchers, entries);
        }
        Expression::AssignmentExpression(a) => scan_expr_for_entries(&a.right, dispatchers, entries),
        Expression::BinaryExpression(b) => {
            scan_expr_for_entries(&b.left, dispatchers, entries);
            scan_expr_for_entries(&b.right, dispatchers, entries);
        }
        Expression::LogicalExpression(l) => {
            scan_expr_for_entries(&l.left, dispatchers, entries);
            scan_expr_for_entries(&l.right, dispatchers, entries);
        }
        Expression::ConditionalExpression(c) => {
            scan_expr_for_entries(&c.test, dispatchers, entries);
            scan_expr_for_entries(&c.consequent, dispatchers, entries);
            scan_expr_for_entries(&c.alternate, dispatchers, entries);
        }
        Expression::SequenceExpression(s) => {
            for e in &s.expressions {
                scan_expr_for_entries(e, dispatchers, entries);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            scan_expr_for_entries(&p.expression, dispatchers, entries);
        }
        Expression::UnaryExpression(u) => scan_expr_for_entries(&u.argument, dispatchers, entries),
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    scan_expr_for_entries(&p.value, dispatchers, entries);
                }
            }
        }
        Expression::StaticMemberExpression(s) => scan_expr_for_entries(&s.object, dispatchers, entries),
        Expression::ComputedMemberExpression(c) => {
            scan_expr_for_entries(&c.object, dispatchers, entries);
            scan_expr_for_entries(&c.expression, dispatchers, entries);
        }
        Expression::NewExpression(n) => {
            scan_expr_for_entries(&n.callee, dispatchers, entries);
            for arg in &n.arguments {
                if let Some(e) = arg.as_expression() {
                    scan_expr_for_entries(e, dispatchers, entries);
                }
            }
        }
        _ => {}
    }
}

fn extract_dispatcher_call(
    call: &oxc_ast::ast::CallExpression<'_>,
    dispatchers: &DoWhileDispatcherMap,
) -> Option<(String, String)> {
    // Direct: DISPATCHER(STATE, args)
    if let Expression::Identifier(callee_id) = &call.callee {
        let name = callee_id.name.as_str();
        if dispatchers.contains_key(name) && call.arguments.len() == 2 {
            if let Some(Expression::Identifier(state_id)) = call.arguments[0].as_expression() {
                return Some((name.to_string(), state_id.name.as_str().to_string()));
            }
        }
    }
    // .call(this, STATE, args) form
    if let Expression::StaticMemberExpression(sme) = &call.callee {
        if sme.property.name.as_str() == "call" {
            if let Expression::Identifier(obj_id) = &sme.object {
                let name = obj_id.name.as_str();
                if dispatchers.contains_key(name) && call.arguments.len() == 3 {
                    if let Some(Expression::Identifier(state_id)) = call.arguments[1].as_expression() {
                        return Some((name.to_string(), state_id.name.as_str().to_string()));
                    }
                }
            }
        }
    }
    None
}

fn compute_reachability(
    dispatchers: &DoWhileDispatcherMap,
    entry_states: &FxHashMap<String, FxHashSet<String>>,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut result: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

    for (name, info) in dispatchers {
        let mut reachable = FxHashSet::default();

        // Seed with entry states
        if let Some(entries) = entry_states.get(name) {
            for entry in entries {
                reachable.insert(entry.clone());
            }
        }
        // Also check alt_name entries
        if let Some(alt) = &info.alt_name {
            if let Some(entries) = entry_states.get(alt.as_str()) {
                for entry in entries {
                    reachable.insert(entry.clone());
                }
            }
        }

        if reachable.is_empty() {
            // No known entry points — conservatively keep all cases
            for case in &info.cases {
                reachable.insert(case.label.clone());
            }
            result.insert(name.clone(), reachable);
            continue;
        }

        // Build transition map: label -> set of successor labels
        let mut transitions: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
        for case in &info.cases {
            let succs = match &case.transition {
                StateTransition::Sequential(next) => vec![next.as_str()],
                StateTransition::Conditional(a, b) => vec![a.as_str(), b.as_str()],
                StateTransition::LoopExit | StateTransition::Return => vec![],
                StateTransition::Unknown => {
                    // Conservative: all other case labels are potential targets
                    info.cases
                        .iter()
                        .filter(|c| c.label != case.label)
                        .map(|c| c.label.as_str())
                        .collect()
                }
            };
            transitions.insert(case.label.as_str(), succs);
        }

        // Fixpoint: expand reachable set by following transitions
        let mut worklist: Vec<String> = reachable.iter().cloned().collect();
        while let Some(label) = worklist.pop() {
            if let Some(succs) = transitions.get(label.as_str()) {
                for &succ in succs {
                    if reachable.insert(succ.to_string()) {
                        worklist.push(succ.to_string());
                    }
                }
            }
        }

        result.insert(name.clone(), reachable);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::dowhile_switch_detector::DoWhileSwitchDetector;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let detector = DoWhileSwitchDetector::new();
        let dispatchers = detector.detect(&program);
        if dispatchers.is_empty() {
            return (Codegen::new().build(&program).code, 0);
        }

        let mut cleaner = DoWhileSwitchCleaner::new(dispatchers, &program);

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);

        let pruned = cleaner.pruned_cases();
        (Codegen::new().build(&program).code, pruned)
    }

    #[test]
    fn prunes_unreachable_case() {
        // Dispatcher with 3 cases: A→B (sequential), B→EXIT (loop exit), C→EXIT (loop exit)
        // Only entry point is A, so A→B are reachable, C is dead.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); s = B; } break;
                        case B: { bar(); s = EXIT; } break;
                        case C: { baz(); s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 1, "should prune 1 dead case (C): {out}");
        assert!(out.contains("case A:"), "case A should be kept: {out}");
        assert!(out.contains("case B:"), "case B should be kept: {out}");
        assert!(!out.contains("case C:"), "case C should be pruned: {out}");
    }

    #[test]
    fn preserves_all_reachable_cases() {
        // All cases reachable: entry=A, A→B, B→C, C→EXIT
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); s = B; } break;
                        case B: { bar(); s = C; } break;
                        case C: { baz(); s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 0, "should prune nothing: {out}");
        assert!(out.contains("case A:"), "case A should be kept: {out}");
        assert!(out.contains("case B:"), "case B should be kept: {out}");
        assert!(out.contains("case C:"), "case C should be kept: {out}");
    }

    #[test]
    fn handles_transitive_reachability() {
        // Entry=A, A→B, B→C, C→EXIT. D is dead.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { s = C; } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 1, "should prune 1 dead case (D): {out}");
        assert!(out.contains("case A:"), "A reachable: {out}");
        assert!(out.contains("case B:"), "B reachable via A: {out}");
        assert!(out.contains("case C:"), "C reachable via A→B: {out}");
        assert!(!out.contains("case D:"), "D unreachable: {out}");
    }

    #[test]
    fn counts_pruned() {
        // Entry=A, A→EXIT. B, C, D are all dead.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = EXIT; } break;
                        case B: { s = EXIT; } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (_, pruned) = run(code);
        assert_eq!(pruned, 3, "should prune 3 dead cases (B, C, D)");
    }

    #[test]
    fn prunes_with_conditional_edges() {
        // A→B (sequential), B→C or D (conditional), entry=A → A,B,C,D reachable. E is dead.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { if (a[0]) { s = C; } else { s = D; } } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                        case E: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 1, "should prune 1 dead case (E): {out}");
        assert!(out.contains("case A:"), "A reachable: {out}");
        assert!(out.contains("case B:"), "B reachable: {out}");
        assert!(out.contains("case C:"), "C reachable via conditional: {out}");
        assert!(out.contains("case D:"), "D reachable via conditional: {out}");
        assert!(!out.contains("case E:"), "E unreachable: {out}");
    }

    #[test]
    fn unknown_preserves_all() {
        // A has Unknown transition (complex body), B and C exist. Entry=A.
        // Unknown conservatively reaches all → nothing pruned.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); } break;
                        case B: { s = EXIT; } break;
                        case C: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 0, "Unknown transition should preserve all: {out}");
        assert!(out.contains("case A:"), "A kept: {out}");
        assert!(out.contains("case B:"), "B kept (reachable via Unknown): {out}");
        assert!(out.contains("case C:"), "C kept (reachable via Unknown): {out}");
    }

    #[test]
    fn bfs_follows_chain() {
        // A→B→C→D→EXIT, entry=A, E is dead.
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { s = C; } break;
                        case C: { s = D; } break;
                        case D: { s = EXIT; } break;
                        case E: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        "#;
        let (out, pruned) = run(code);
        assert_eq!(pruned, 1, "should prune 1 dead case (E): {out}");
        assert!(out.contains("case A:"), "A reachable: {out}");
        assert!(out.contains("case B:"), "B reachable: {out}");
        assert!(out.contains("case C:"), "C reachable: {out}");
        assert!(out.contains("case D:"), "D reachable: {out}");
        assert!(!out.contains("case E:"), "E unreachable: {out}");
    }
}
