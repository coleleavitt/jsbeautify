//! Bindings whose value is in place before any code can read them.
//!
//! `var P = function () { … }` holds `undefined` until its declarator runs,
//! and a `let` or `const` throws until then. A pass that replaces a call
//! `P(…)` with what the function computes, or deletes `P`'s declaration, turns
//! a call that would have thrown into one that does not. It may only do so when
//! no call can run before the binding is set.

use oxc_ast::AstKind;
use oxc_ast::ast::{
    ArrayExpressionElement, ArrowFunctionExpression, AssignmentExpression, AssignmentOperator, AssignmentTarget,
    BindingPattern, BlockStatement, Expression, FormalParameter, Function, FunctionBody, IdentifierReference, Program,
    Statement, StaticBlock, SwitchCase, SwitchStatement, VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::annex_b::annex_b_shadowed_symbols;
use crate::ast_deobfuscate::rename_guards::is_global_binding;

/// Bindings set by a statement at the top level of a statement list (a
/// function body, a block, a static block or the program) before any code
/// that could read them runs.
///
/// The statement is a `var`, `let` or `const` declarator, or a plain `X = …`
/// assignment to a `var` (not a parameter, which `arguments` reaches), whose
/// value runs no code: a function, a literal or an array of literals. The
/// binding counts when every reference to it lies in that list and no code
/// that can run before the statement references it. That code is the earlier
/// statements of the list with every function they create, and, followed
/// transitively, every function a binding they reference names (by a
/// declaration, a declarator or an assignment). In a classic script, earlier
/// code may also reach any function bound to a global or an Annex B block
/// function without naming it (`globalThis.x()`, another script), so all of
/// those count as referenced; Annex B functions count in any program.
///
/// A module that imports or re-exports another module is left out at its top
/// level, as that module may call back into it before its body runs. The
/// analysis does a bounded amount of work per byte of source; past that, no
/// further binding counts.
#[must_use]
pub fn initialized_early(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    let mut owners = Owners {
        scoping,
        owner: None,
        declared_as: None,
        tick: 0,
        references: FxHashMap::default(),
        extents: FxHashMap::default(),
        lists: FxHashMap::default(),
        params: FxHashSet::default(),
    };
    owners.visit_program(program);
    let annex_b = annex_b_shadowed_symbols(program, scoping);
    // Everything a function earlier code may call without naming it can
    // reach, closed over once per program.
    let mut global_reach = Reached {
        references: &owners.references,
        global_reach: None,
        symbols: FxHashSet::default(),
        expanded: FxHashSet::default(),
        unnamed_calls: false,
        escaped: Vec::new(),
    };
    let global_owners: Vec<SymbolId> = owners
        .references
        .keys()
        .copied()
        .filter(|&symbol| annex_b.contains(&symbol) || is_global_binding(program.source_type, scoping, symbol))
        .collect();
    let mut budget = 8 * program.span.size() as usize;
    global_reach.extend(global_owners, &mut budget);
    let global_reach = global_reach.symbols;
    let mut early = Early {
        scoping,
        owners: &owners,
        global_reach: &global_reach,
        found: FxHashSet::default(),
        budget,
    };
    let requests_modules = program.body.iter().any(|stmt| {
        matches!(
            stmt,
            Statement::ImportDeclaration(_) | Statement::ExportAllDeclaration(_) | Statement::ExportFromDeclaration(_)
        )
    });
    if !requests_modules {
        early.body(&program.body);
    }
    early.visit_program(program);
    early.found
}

/// Whether every call through `symbol` reaches its value: it is a hoisted
/// function declaration, or it is in `early` ([`initialized_early`]).
#[must_use]
pub fn defined_before_calls(scoping: &Scoping, early: &FxHashSet<SymbolId>, symbol: SymbolId) -> bool {
    scoping.symbol_flags(symbol).is_function() || early.contains(&symbol)
}

/// Evaluating `expr` runs no code and cannot throw.
fn is_inert_value(expr: &Expression<'_>) -> bool {
    let literal = |expr: &Expression<'_>| {
        matches!(
            expr,
            Expression::NumericLiteral(_)
                | Expression::StringLiteral(_)
                | Expression::BooleanLiteral(_)
                | Expression::NullLiteral(_)
        )
    };
    match expr {
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => true,
        Expression::ArrayExpression(array) => array.elements.iter().all(|element| {
            matches!(element, ArrayExpressionElement::Elision(_)) || element.as_expression().is_some_and(literal)
        }),
        expr => literal(expr),
    }
}

/// The key of a statement list: where its statements are stored.
fn list_key(body: &[Statement<'_>]) -> usize {
    body.as_ptr().addr()
}

/// One walk over the program that records, in visit order, where each
/// binding is referenced and which statement lists contain which references.
struct Owners<'s> {
    scoping: &'s Scoping,
    owner: Option<SymbolId>,
    /// The binding a function about to be visited is declared or assigned as.
    declared_as: Option<SymbolId>,
    /// Counts references and list boundaries in visit order.
    tick: u32,
    /// For each binding that names a function, the bindings that function's
    /// body references, including from the unnamed functions it creates.
    references: FxHashMap<SymbolId, Vec<SymbolId>>,
    /// Per binding, the first and last tick of its references.
    extents: FxHashMap<SymbolId, (u32, u32)>,
    /// Per statement list, the ticks it spans.
    lists: FxHashMap<usize, (u32, u32)>,
    params: FxHashSet<SymbolId>,
}

impl Owners<'_> {
    fn with_owner(&mut self, owner: Option<SymbolId>, visit: impl FnOnce(&mut Self)) {
        let outer = self.owner;
        self.owner = owner.or(outer);
        visit(self);
        self.owner = outer;
    }

    fn list(&mut self, body: &[Statement<'_>], visit: impl FnOnce(&mut Self)) {
        self.tick += 1;
        let start = self.tick;
        visit(self);
        self.tick += 1;
        self.lists.insert(list_key(body), (start, self.tick));
    }

    const fn named_function(expr: &Expression<'_>) -> bool {
        matches!(
            expr,
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
        )
    }
}

impl<'a> Visit<'a> for Owners<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.list(&program.body, |this| walk::walk_program(this, program));
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.list(&body.statements, |this| walk::walk_function_body(this, body));
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        self.list(&block.body, |this| walk::walk_block_statement(this, block));
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.list(&block.body, |this| walk::walk_static_block(this, block));
    }

    fn visit_switch_case(&mut self, case: &SwitchCase<'a>) {
        self.list(&case.consequent, |this| walk::walk_switch_case(this, case));
    }

    fn visit_formal_parameter(&mut self, param: &FormalParameter<'a>) {
        self.params.extend(
            param
                .pattern
                .get_binding_identifiers()
                .iter()
                .filter_map(|id| id.symbol_id.get()),
        );
        walk::walk_formal_parameter(self, param);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        self.visit_binding_pattern(&declarator.id);
        if let Some(init) = &declarator.init {
            self.declared_as = match &declarator.id {
                BindingPattern::BindingIdentifier(id) if Self::named_function(init) => id.symbol_id.get(),
                _ => None,
            };
            self.visit_expression(init);
        }
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        self.visit_assignment_target(&assign.left);
        self.declared_as = match &assign.left {
            AssignmentTarget::AssignmentTargetIdentifier(target) if Self::named_function(&assign.right) => target
                .reference_id
                .get()
                .and_then(|reference| self.scoping.get_reference(reference).symbol_id()),
            _ => None,
        };
        self.visit_expression(&assign.right);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        let named = self.declared_as.take();
        let named = named.or_else(|| {
            func.is_declaration()
                .then(|| func.id.as_ref().and_then(|id| id.symbol_id.get()))
                .flatten()
        });
        self.with_owner(named, |this| walk::walk_function(this, func, flags));
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        let named = self.declared_as.take();
        self.with_owner(named, |this| walk::walk_arrow_function_expression(this, arrow));
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        let Some(symbol) = ident
            .reference_id
            .get()
            .and_then(|reference| self.scoping.get_reference(reference).symbol_id())
        else {
            return;
        };
        self.tick += 1;
        let tick = self.tick;
        self.extents
            .entry(symbol)
            .and_modify(|(first, last)| {
                *first = (*first).min(tick);
                *last = (*last).max(tick);
            })
            .or_insert((tick, tick));
        if let Some(owner) = self.owner {
            self.references.entry(owner).or_default().push(symbol);
        }
    }
}

struct Early<'s, 'o> {
    scoping: &'s Scoping,
    owners: &'o Owners<'s>,
    /// The bindings functions earlier code may call without naming them
    /// can reach.
    global_reach: &'o FxHashSet<SymbolId>,
    found: FxHashSet<SymbolId>,
    budget: usize,
}

impl<'s, 'o> Early<'s, 'o> {
    fn reached(&self) -> Reached<'o> {
        let owners: &'o Owners<'s> = self.owners;
        Reached {
            references: &owners.references,
            global_reach: Some(self.global_reach),
            symbols: FxHashSet::default(),
            expanded: FxHashSet::default(),
            unnamed_calls: false,
            escaped: Vec::new(),
        }
    }

    /// Records the bindings `body`'s top-level statements set before code
    /// that could read them runs.
    fn body(&mut self, body: &[Statement<'_>]) {
        if body.iter().any(|stmt| self.sets_inert_value(stmt)) {
            let mut reached = self.reached();
            self.list(body, &mut reached, false);
        }
    }

    /// Runs `body` after the code `reached` stands for, recording the
    /// bindings it sets early. With `to_end`, every statement is added to
    /// `reached`, for code that may run after it (a later `case`).
    fn list(&mut self, body: &[Statement<'_>], reached: &mut Reached<'_>, to_end: bool) {
        let last_setter = body.iter().rposition(|stmt| self.sets_inert_value(stmt));
        let end = if to_end {
            body.len()
        } else {
            last_setter.map_or(0, |last| last + 1)
        };
        let Some(&list) = self.owners.lists.get(&list_key(body)) else {
            return;
        };
        for stmt in &body[..end] {
            match stmt {
                Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {}
                Statement::VariableDeclaration(decl)
                    if matches!(
                        decl.kind,
                        VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
                    ) =>
                {
                    for declarator in &decl.declarations {
                        self.declarator(declarator, list, reached);
                    }
                }
                Statement::ExpressionStatement(stmt) if let Some(symbol) = self.inert_assignment(&stmt.expression) => {
                    self.set(symbol, list, reached);
                }
                stmt => self.run(reached, |collect| collect.visit_statement(stmt)),
            }
            if self.budget == 0 {
                return;
            }
        }
    }

    fn declarator(&mut self, declarator: &VariableDeclarator<'_>, list: (u32, u32), reached: &mut Reached<'_>) {
        match (&declarator.id, &declarator.init) {
            (BindingPattern::BindingIdentifier(_), None) => {}
            (BindingPattern::BindingIdentifier(id), Some(init)) if is_inert_value(init) => {
                if let Some(symbol) = id.symbol_id.get() {
                    self.set(symbol, list, reached);
                }
            }
            _ => self.run(reached, |collect| collect.visit_variable_declarator(declarator)),
        }
    }

    /// Records `symbol`, set by a statement of `list`, when every reference
    /// to it is in `list` and no code of `list` that ran before may read it.
    ///
    /// When a reference lies outside `list`, code there may read the value
    /// set here once control leaves `list`, also when it stops before a later
    /// statement (a `return`, `break` or `throw`). The function it holds then
    /// counts as reached from the next code that runs on.
    fn set(&mut self, symbol: SymbolId, (start, end): (u32, u32), reached: &mut Reached<'_>) {
        let inside = self
            .owners
            .extents
            .get(&symbol)
            .is_none_or(|&(first, last)| start <= first && last <= end);
        if !inside {
            reached.escaped.push(symbol);
        } else if self.budget > 0 && !self.owners.params.contains(&symbol) && !reached.contains(symbol) {
            self.found.insert(symbol);
        }
    }

    /// Adds the bindings the code `visit` walks may reference when it runs.
    fn run(&mut self, reached: &mut Reached<'_>, visit: impl FnOnce(&mut Collect<'_>)) {
        let mut collect = Collect {
            scoping: self.scoping,
            symbols: Vec::new(),
            budget: self.budget,
        };
        visit(&mut collect);
        self.budget = collect.budget;
        collect.symbols.append(&mut reached.escaped);
        // Any code may call a function bound to a global, or an Annex B
        // block function, without naming it.
        reached.unnamed_calls = true;
        reached.extend(collect.symbols, &mut self.budget);
    }

    fn sets_inert_value(&self, stmt: &Statement<'_>) -> bool {
        match stmt {
            Statement::VariableDeclaration(decl) => decl.declarations.iter().any(|declarator| {
                matches!(declarator.id, BindingPattern::BindingIdentifier(_))
                    && declarator.init.as_ref().is_some_and(|init| is_inert_value(init))
            }),
            Statement::ExpressionStatement(stmt) => self.inert_assignment(&stmt.expression).is_some(),
            _ => false,
        }
    }

    /// The `var` `expr` assigns a value that runs no code.
    fn inert_assignment(&self, expr: &Expression<'_>) -> Option<SymbolId> {
        let Expression::AssignmentExpression(assign) = expr else {
            return None;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return None;
        };
        let symbol = self.scoping.get_reference(target.reference_id.get()?).symbol_id()?;
        (assign.operator == AssignmentOperator::Assign
            && is_inert_value(&assign.right)
            && self.scoping.symbol_flags(symbol).is_function_scoped_declaration()
            && !self.owners.params.contains(&symbol))
        .then_some(symbol)
    }
}

impl<'a> Visit<'a> for Early<'_, '_> {
    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        if self.budget > 0 {
            self.body(&body.statements);
        }
        walk::walk_function_body(self, body);
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        if self.budget > 0 {
            self.body(&block.body);
        }
        walk::walk_block_statement(self, block);
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        if self.budget > 0 {
            self.body(&block.body);
        }
        walk::walk_static_block(self, block);
    }

    /// A case's statements run after the discriminant, possibly every case
    /// test, the statements of the cases before it (falling through), and any
    /// function declared in the switch.
    fn visit_switch_statement(&mut self, switch: &SwitchStatement<'a>) {
        if self.budget > 0
            && switch
                .cases
                .iter()
                .any(|case| case.consequent.iter().any(|stmt| self.sets_inert_value(stmt)))
        {
            let mut reached = self.reached();
            self.run(&mut reached, |collect| {
                collect.visit_expression(&switch.discriminant);
                for test in switch.cases.iter().filter_map(|case| case.test.as_ref()) {
                    collect.visit_expression(test);
                }
            });
            // A function declared in any case is hoisted to the whole switch,
            // so a later case jumped to directly may call it first.
            let hoisted = switch
                .cases
                .iter()
                .flat_map(|case| &case.consequent)
                .filter_map(|stmt| match stmt {
                    Statement::FunctionDeclaration(func) => func.id.as_ref()?.symbol_id.get(),
                    _ => None,
                })
                .collect();
            reached.extend(hoisted, &mut self.budget);
            for case in &switch.cases {
                if self.budget == 0 {
                    break;
                }
                self.list(&case.consequent, &mut reached, true);
            }
        }
        walk::walk_switch_statement(self, switch);
    }
}

/// The bindings code run so far may have referenced, closed over the named
/// functions among them.
struct Reached<'r> {
    references: &'r FxHashMap<SymbolId, Vec<SymbolId>>,
    /// What any code reaches through functions it may call without naming
    /// them, counted once `unnamed_calls` is set.
    global_reach: Option<&'r FxHashSet<SymbolId>>,
    symbols: FxHashSet<SymbolId>,
    expanded: FxHashSet<SymbolId>,
    /// Code ran, so `global_reach` counts as reached.
    unnamed_calls: bool,
    /// Bindings set to a function that code outside the list may call once
    /// control leaves it, added when the next code runs.
    escaped: Vec<SymbolId>,
}

impl Reached<'_> {
    fn in_global_reach(&self, symbol: SymbolId) -> bool {
        self.unnamed_calls && self.global_reach.is_some_and(|reach| reach.contains(&symbol))
    }

    fn contains(&self, symbol: SymbolId) -> bool {
        self.symbols.contains(&symbol) || self.in_global_reach(symbol)
    }

    fn extend(&mut self, symbols: Vec<SymbolId>, budget: &mut usize) {
        let mut pending = symbols;
        while let Some(symbol) = pending.pop() {
            if self.in_global_reach(symbol) {
                continue;
            }
            self.symbols.insert(symbol);
            if let Some(referenced) = self.references.get(&symbol)
                && self.expanded.insert(symbol)
            {
                *budget = budget.saturating_sub(referenced.len());
                if *budget == 0 {
                    return;
                }
                pending.extend(referenced.iter().copied().filter(|&s| !self.contains(s)));
            }
        }
    }
}

/// The bindings a piece of code references, including in the functions it
/// creates.
struct Collect<'s> {
    scoping: &'s Scoping,
    symbols: Vec<SymbolId>,
    budget: usize,
}

impl<'a> Visit<'a> for Collect<'_> {
    fn enter_node(&mut self, _kind: AstKind<'a>) {
        self.budget = self.budget.saturating_sub(1);
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if let Some(symbol) = ident
            .reference_id
            .get()
            .and_then(|reference| self.scoping.get_reference(reference).symbol_id())
        {
            self.symbols.push(symbol);
        }
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::initialized_early;
    use crate::ast_deobfuscate::scaling::assert_near_linear;
    use crate::ast_deobfuscate::semantic;

    /// The names of the bindings `initialized_early` returns, sorted.
    fn early(code: &str, source_type: SourceType) -> Vec<String> {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, source_type).parse().program;
        let scoping = semantic::scoping(&program);
        let mut names: Vec<_> = initialized_early(&program, &scoping)
            .into_iter()
            .map(|symbol| scoping.symbol_name(symbol).to_string())
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn bindings_set_by_leading_statements_that_run_no_code() {
        let code = "var f = function (a) { return a; }, n = 1, u; function g() { let h = (a) => a; var k; \
                    function m() {} k = [1, 'x']; h(k); var late = h; }";
        assert_eq!(early(code, SourceType::cjs()), ["f", "h", "k", "n"]);
    }

    #[test]
    fn code_that_cannot_reach_the_binding_may_run_first() {
        for code in [
            // The Akamai BMP prologue: setup functions that touch other bindings.
            "YY(); function YY() { cY = []['keys'](); } Yst(); function Yst() { a = 1 + 2; } \
             var f = function (a) { return a; }; function main() { return f(1); }",
            "var n = h(), f = function (a) { return a; };",
            "class C {} var f = function (a) { return a; };",
            // Stored where later code may read it, with no code before `f` is set.
            "var x; function g() { x = function () { return f(1); }; var f = function (a) { return a; }; } x();",
            // In a block or case run before every reference, also after earlier
            // cases' tests and statements that do not reach it.
            "function g(x) { switch (x) { case 0: h(); case 1: var f = function (a) { return a; }; return f(1); } }",
            "function g(x) { switch (x) { case 1: { h(); var f = function (a) { return a; }; return f(1); } } }",
            "function g() { for (;;) { h(); var f; f = function (a) { return a; }; f(1); } }",
        ] {
            assert!(early(code, SourceType::cjs()).iter().any(|name| name == "f"), "{code}");
        }
    }

    #[test]
    fn bindings_code_may_read_first_are_late() {
        for code in [
            "try { f(1); } catch (e) {} var f = function (a) { return a; };",
            "g(); let f = function (a) { return a; }; function g() { return f(1); }",
            // Through named functions, transitively.
            "g(); function g() { h(); } function h() { f(1); } var f = function (a) { return a; };",
            "var a = function () { return f(1); }; a(); var f = function (b) { return b; };",
            // Through functions passed on or created by that code.
            "g(); function g() { [h].forEach((x) => x()); } function h() { f(1); } var f = function (a) { return a; };",
            "foo(function () { f(1); }); var f = function (a) { return a; };",
            "var o = { m() { return f(1); } }; o.m(); var f = function (a) { return a; };",
            // Referenced outside the statement list that sets it.
            "if (x) { var f = function (a) { return a; }; } f(1);",
            "switch (x) { case 1: var f = function (a) { return a; }; case 2: f(1); }",
            // Function declarations and case tests of a switch run before its
            // cases, and a later case may be jumped to first.
            "'use strict'; function r(n) { switch (n) { case 1: var f = function (a) { return a; }; function g() { f(1); } break; case 0: g(); } }",
            "function r(n) { switch (n) { case 0: g(); case 1: var f = function (a) { return a; }; function g() { f(1); } } }",
            "function r(n) { switch (n) { case g(): break; case 1: var f = function (a) { return a; }; function g() { f(1); } } }",
            // Read later through an outer binding, after an early `return`.
            "var x; function g(c) { x = function () { return f(1); }; if (c) return; var f = function (a) { return a; }; } x();",
            // Read again by an earlier statement of a loop body.
            "for (;;) { f(1); var f = function (a) { return a; }; }",
            "var f; function g() { f = function (a) { return a; }; } f(1);",
            "var [f] = [function (a) { return a; }];",
        ] {
            assert!(!early(code, SourceType::cjs()).iter().any(|name| name == "f"), "{code}");
        }
    }

    #[test]
    fn parameters_are_reachable_through_arguments() {
        let code =
            "function o(p) { p = function () { return f(1); }; arguments[0](); var f = function (a) { return a; }; }";
        let found = early(code, SourceType::cjs());
        assert!(!found.iter().any(|name| name == "f" || name == "p"), "{found:?}");
    }

    #[test]
    fn script_globals_are_reachable_without_a_name() {
        let code = "{ var x = function () { return f(1); }; globalThis.x(); let f = function (a) { return a; }; }";
        assert!(!early(code, SourceType::script()).iter().any(|name| name == "f"));
        assert!(early(code, SourceType::cjs()).iter().any(|name| name == "f"));
        // A call may reach an Annex B block function through the `var` it
        // also assigns, whichever binding the call resolves to.
        let code = "function g() { { function x() { return f(1); } } x(); let f = (a) => a; }";
        assert!(!early(code, SourceType::cjs()).iter().any(|name| name == "f"));
    }

    #[test]
    fn imports_may_call_back_before_the_module_body_runs() {
        let code = "var f = function (a) { return a; }; export function g() { return f(1); }";
        assert_eq!(early(code, SourceType::mjs()), ["f"]);
        for request in [
            "import './cycle.js';",
            "export * from './cycle.js';",
            "export { h } from './cycle.js';",
        ] {
            assert_eq!(
                early(&format!("{request} {code}"), SourceType::mjs()),
                Vec::<String>::new(),
                "{request}"
            );
        }
    }

    #[test]
    fn script_globals_are_closed_over_once() {
        // Every block runs code, so each one reaches what the global functions
        // reach; that closure must not be recomputed per block.
        assert_near_linear("initialized_early", 1000, |n| {
            let code = (0..n)
                .map(|i| {
                    format!(
                        "function g{i}() {{ return g{}; }} {{ h(); var v{i} = function () {{}}; }}\n",
                        i + 1
                    )
                })
                .collect::<Vec<_>>()
                .concat();
            let allocator = Allocator::default();
            let program = Parser::new(&allocator, &code, SourceType::script()).parse().program;
            let scoping = semantic::scoping(&program);
            let started = std::time::Instant::now();
            let early = initialized_early(&program, &scoping);
            let elapsed = started.elapsed();
            assert_eq!(early.len(), n);
            elapsed
        });
    }
}
