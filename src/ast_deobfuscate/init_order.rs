//! Bindings whose value is in place before any code can read them.
//!
//! `var P = function () { … }` holds `undefined` until its declarator runs,
//! and a `let` or `const` throws until then. A pass that replaces a call
//! `P(…)` with what the function computes, or deletes `P`'s declaration, turns
//! a call that would have thrown into one that does not. It may only do so when
//! no call can run before the binding is set.

use oxc_ast::AstKind;
use oxc_ast::ast::{
    ArrayExpressionElement, ArrowFunctionExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    BlockStatement, Expression, Function, FunctionBody, IdentifierReference, Program, Statement, StaticBlock,
    SwitchCase, VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Bindings set by a statement at the top level of a function body or the
/// program before any code that could read them runs.
///
/// The statement is a `var`, `let` or `const` declarator, or a plain `X = …`
/// assignment to a `var` of that scope, whose value runs no code: a function,
/// a literal or an array of literals. The binding counts when no code that
/// can run before that statement references it: the earlier statements of
/// the list, with every function they create, and transitively every named
/// function that code references (called or passed on). No other code can
/// reach a binding of the list before the statement runs.
///
/// A module with imports is left out, as a module it imports may call back
/// into it before its body runs. The analysis does a bounded amount of work
/// per byte of source; past that, no further binding counts.
#[must_use]
pub fn initialized_early(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    let mut owners = Owners {
        scoping,
        owner: None,
        declared_as: None,
        references: FxHashMap::default(),
        extents: FxHashMap::default(),
    };
    owners.visit_program(program);
    let mut early = Early {
        scoping,
        references: &owners.references,
        extents: &owners.extents,
        found: FxHashSet::default(),
        budget: 8 * program.span.size() as usize,
    };
    if !program
        .body
        .iter()
        .any(|stmt| matches!(stmt, Statement::ImportDeclaration(_)))
    {
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

/// For each function a binding names (a declaration, or a declarator
/// initialised to a function or an arrow), the bindings its body references,
/// including from the unnamed functions it creates.
struct Owners<'s> {
    scoping: &'s Scoping,
    owner: Option<SymbolId>,
    /// The binding of the declarator whose initialiser is being visited.
    declared_as: Option<SymbolId>,
    references: FxHashMap<SymbolId, Vec<SymbolId>>,
    /// Per binding, the source range its references span.
    extents: FxHashMap<SymbolId, Span>,
}

impl Owners<'_> {
    fn with_owner(&mut self, owner: Option<SymbolId>, visit: impl FnOnce(&mut Self)) {
        let outer = self.owner;
        self.owner = owner.or(outer);
        visit(self);
        self.owner = outer;
    }
}

impl<'a> Visit<'a> for Owners<'_> {
    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        self.visit_binding_pattern(&declarator.id);
        if let Some(init) = &declarator.init {
            self.declared_as = match (&declarator.id, init) {
                (
                    BindingPattern::BindingIdentifier(id),
                    Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_),
                ) => id.symbol_id.get(),
                _ => None,
            };
            self.visit_expression(init);
        }
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
        let Some(symbol) = self.scoping.get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        self.extents
            .entry(symbol)
            .and_modify(|extent| *extent = extent.merge(ident.span))
            .or_insert(ident.span);
        if let Some(owner) = self.owner {
            self.references.entry(owner).or_default().push(symbol);
        }
    }
}

struct Early<'s, 'r> {
    scoping: &'s Scoping,
    references: &'r FxHashMap<SymbolId, Vec<SymbolId>>,
    extents: &'r FxHashMap<SymbolId, Span>,
    found: FxHashSet<SymbolId>,
    budget: usize,
}

impl Early<'_, '_> {
    /// Records the bindings `body`'s top-level statements set before code
    /// that could read them runs.
    fn body(&mut self, body: &[Statement<'_>]) {
        let Some(last_setter) = body.iter().rposition(|stmt| self.sets_inert_value(stmt)) else {
            return;
        };
        let (Some(first), Some(end)) = (body.first(), body.last()) else {
            return;
        };
        let list = Span::new(first.span().start, end.span().end);
        let mut reached = Reached {
            references: self.references,
            symbols: FxHashSet::default(),
            expanded: FxHashSet::default(),
        };
        for stmt in &body[..=last_setter] {
            match stmt {
                Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {}
                Statement::VariableDeclaration(decl)
                    if matches!(
                        decl.kind,
                        VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
                    ) =>
                {
                    for declarator in &decl.declarations {
                        self.declarator(declarator, list, &mut reached);
                    }
                }
                Statement::ExpressionStatement(stmt) if let Some(symbol) = self.inert_assignment(&stmt.expression) => {
                    self.set(symbol, list, &reached);
                }
                stmt => self.run(&mut reached, |collect| collect.visit_statement(stmt)),
            }
            if self.budget == 0 {
                return;
            }
        }
    }

    fn declarator(&mut self, declarator: &VariableDeclarator<'_>, list: Span, reached: &mut Reached<'_>) {
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
    fn set(&mut self, symbol: SymbolId, list: Span, reached: &Reached<'_>) {
        let inside = self
            .extents
            .get(&symbol)
            .is_none_or(|extent| list.contains_inclusive(*extent));
        if self.budget > 0 && inside && !reached.symbols.contains(&symbol) {
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
        reached.extend(collect.symbols, &mut self.budget);
    }

    fn sets_inert_value(&self, stmt: &Statement<'_>) -> bool {
        match stmt {
            Statement::VariableDeclaration(decl) => decl.declarations.iter().any(|declarator| {
                matches!(declarator.id, BindingPattern::BindingIdentifier(_))
                    && declarator.init.as_ref().is_some_and(is_inert_value)
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
        let symbol = self.scoping.get_reference(target.reference_id()).symbol_id()?;
        (assign.operator == AssignmentOperator::Assign
            && is_inert_value(&assign.right)
            && self.scoping.symbol_flags(symbol).is_function_scoped_declaration())
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

    fn visit_switch_case(&mut self, case: &SwitchCase<'a>) {
        if self.budget > 0 {
            self.body(&case.consequent);
        }
        walk::walk_switch_case(self, case);
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        if self.budget > 0 {
            self.body(&block.body);
        }
        walk::walk_static_block(self, block);
    }
}

/// The bindings code run so far may have referenced, closed over the named
/// functions among them.
struct Reached<'r> {
    references: &'r FxHashMap<SymbolId, Vec<SymbolId>>,
    symbols: FxHashSet<SymbolId>,
    expanded: FxHashSet<SymbolId>,
}

impl Reached<'_> {
    fn extend(&mut self, symbols: Vec<SymbolId>, budget: &mut usize) {
        let mut pending = symbols;
        while let Some(symbol) = pending.pop() {
            self.symbols.insert(symbol);
            if let Some(referenced) = self.references.get(&symbol)
                && self.expanded.insert(symbol)
            {
                *budget = budget.saturating_sub(referenced.len());
                if *budget == 0 {
                    return;
                }
                pending.extend(referenced.iter().copied().filter(|s| !self.symbols.contains(s)));
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
        if let Some(symbol) = self.scoping.get_reference(ident.reference_id()).symbol_id() {
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
            // In a block or case run before every reference.
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
            // Read again by an earlier statement of a loop body.
            "for (;;) { f(1); var f = function (a) { return a; }; }",
            "var f; function g() { f = function (a) { return a; }; } f(1);",
            "var [f] = [function (a) { return a; }];",
        ] {
            assert!(!early(code, SourceType::cjs()).iter().any(|name| name == "f"), "{code}");
        }
    }

    #[test]
    fn imports_may_call_back_before_the_module_body_runs() {
        let code = "var f = function (a) { return a; }; export function g() { return f(1); }";
        assert_eq!(early(code, SourceType::mjs()), ["f"]);
        assert_eq!(
            early(&format!("import './cycle.js'; {code}"), SourceType::mjs()),
            Vec::<String>::new()
        );
    }
}
