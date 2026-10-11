//! Reads of a function's own call context.
//!
//! `this`, `arguments`, `new.target` and `super` are bound per non-arrow
//! function; arrows (and direct `eval`) see the enclosing function's bindings.
//! A rewrite that moves code into another function, or changes how a function
//! is called, preserves behavior only for the context values it keeps intact.

use oxc_ast::ast::{
    BindingPattern, CallExpression, Expression, Function, IdentifierReference, NewTarget, Program, Statement, Super,
    ThisExpression, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::rename_guards::is_global_binding;

/// Context reads found in one function context, nested arrows included and
/// nested non-arrow functions excluded.
#[derive(Default, Clone, Copy)]
pub struct ContextReads {
    /// `this` or `super`.
    pub this: bool,
    /// `arguments` or `new.target`.
    pub arguments: bool,
    /// A direct `eval(...)`, which can read any of the above.
    pub eval: bool,
}

impl ContextReads {
    /// Reads of `func`'s own context from its parameters and body.
    #[must_use]
    pub fn of_function(func: &Function<'_>) -> Self {
        let mut reads = Self::default();
        reads.visit_formal_parameters(&func.params);
        if let Some(body) = &func.body {
            reads.visit_function_body(body);
        }
        reads
    }

    /// Reads of the enclosing function's context from `stmts`.
    #[must_use]
    pub fn of_statements(stmts: &[Statement<'_>]) -> Self {
        let mut reads = Self::default();
        for stmt in stmts {
            reads.visit_statement(stmt);
        }
        reads
    }

    /// Whether the `this` value can be observed.
    #[must_use]
    pub const fn observes_this(self) -> bool {
        self.this || self.eval
    }

    /// Whether any context value can be observed.
    #[must_use]
    pub const fn any(self) -> bool {
        self.this || self.arguments || self.eval
    }
}

impl<'a> Visit<'a> for ContextReads {
    fn visit_function(&mut self, _it: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_this_expression(&mut self, _it: &ThisExpression) {
        self.this = true;
    }

    fn visit_super(&mut self, _it: &Super) {
        self.this = true;
    }

    fn visit_new_target(&mut self, _it: &NewTarget) {
        self.arguments = true;
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if it.name == "arguments" {
            self.arguments = true;
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(id) = it.callee.without_parentheses()
            && id.name == "eval"
        {
            self.eval = true;
        }
        walk::walk_call_expression(self, it);
    }
}

/// Never-reassigned, file-local bindings whose value is a function that
/// ignores the `this` it is called with: arrows, and functions that never
/// observe `this`. Calling such a binding with any receiver behaves like a
/// plain call.
#[must_use]
pub fn this_free_functions(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    let mut functions = ThisFreeFunctions::default();
    functions.visit_program(program);
    functions
        .0
        .into_iter()
        .filter(|&symbol| {
            !scoping.symbol_is_mutated(symbol)
                && scoping.symbol_redeclarations(symbol).is_empty()
                && !is_global_binding(program.source_type, scoping, symbol)
        })
        .collect()
}

#[derive(Default)]
struct ThisFreeFunctions(FxHashSet<SymbolId>);

impl<'a> Visit<'a> for ThisFreeFunctions {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.is_declaration()
            && let Some(symbol) = it.id.as_ref().and_then(|id| id.symbol_id.get())
            && !ContextReads::of_function(it).observes_this()
        {
            self.0.insert(symbol);
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &it.id
            && let Some(symbol) = id.symbol_id.get()
        {
            let this_free = match it.init.as_ref().map(Expression::without_parentheses) {
                Some(Expression::ArrowFunctionExpression(_)) => true,
                Some(Expression::FunctionExpression(func)) => !ContextReads::of_function(func).observes_this(),
                _ => false,
            };
            if this_free {
                self.0.insert(symbol);
            }
        }
        walk::walk_variable_declarator(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn reads_of_first_function(code: &str) -> ContextReads {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        let Some(Statement::FunctionDeclaration(func)) = program.body.first() else {
            panic!("expected a function declaration: {code}");
        };
        ContextReads::of_function(func)
    }

    #[test]
    fn arrows_share_the_context_and_functions_do_not() {
        assert!(reads_of_first_function("function f() { return () => this.x; }").observes_this());
        assert!(!reads_of_first_function("function f() { return function () { return this; }; }").any());
        assert!(reads_of_first_function("function f() { return () => arguments[0]; }").arguments);
        assert!(reads_of_first_function("function f() { return () => new.target; }").arguments);
    }

    #[test]
    fn parameters_classes_and_eval_count() {
        assert!(reads_of_first_function("function f(a = this) { return a; }").observes_this());
        assert!(reads_of_first_function("function f() { return class extends this.B {}; }").observes_this());
        assert!(reads_of_first_function("function f(s) { return (eval)(s); }").observes_this());
        assert!(!reads_of_first_function("function f(s) { return (0, eval)(s); }").any());
        assert!(!reads_of_first_function("function f(a) { return a.this + a.arguments; }").any());
    }
}
