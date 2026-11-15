use oxc_allocator::{CloneIn, GetAllocator};
use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, Expression, Function, IdentifierReference, Program, Statement,
};
use oxc_semantic::Scoping;
use oxc_span::SPAN;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::redirectable_symbols;

use super::{Ctx, only_local_access};

/// Inlines calls of two-argument comparison proxies as binary expressions.
///
/// Collects `var NAME = function(a, b) { return a <op> b; };` and
/// `function NAME(a, b) { return a <op> b; }` at the top level or in the body
/// of a top-level IIFE, where `<op>` is one of `===`, `!==`, `==`, `!=`, `<`,
/// `>`, `<=`, `>=`, and rewrites each call `NAME(x, y)` that resolves to such a
/// binding into `x <op> y`. This unwraps the BMP-specific
/// `BI(typeof x, "undefined")` idiom to `typeof x === "undefined"`.
///
/// Proxies and call sites are matched by `SymbolId`. A binding is a proxy only
/// when nothing else can reach it at runtime: it is declared once, never
/// written, not a classic-script global, not visible to a direct `eval`, and
/// not shadowed by a `with` object or an Annex B block function.
pub struct EqualityProxyUnwrapper {
    proxies: FxHashMap<SymbolId, BinaryOperator>,
    unwrapped: usize,
}

impl EqualityProxyUnwrapper {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proxies: FxHashMap::default(),
            unwrapped: 0,
        }
    }

    #[must_use]
    pub const fn unwrapped_count(&self) -> usize {
        self.unwrapped
    }

    #[must_use]
    pub const fn proxies(&self) -> &FxHashMap<SymbolId, BinaryOperator> {
        &self.proxies
    }

    /// Scan the program for equality-proxy definitions. Call before traversal,
    /// with the scoping the traversal uses.
    pub fn collect(&mut self, program: &Program<'_>, scoping: &Scoping) {
        for stmt in &program.body {
            self.scan_stmt(stmt, scoping);
        }
        if self.proxies.is_empty() {
            return;
        }
        let redirectable = redirectable_symbols(program, scoping);
        self.proxies.retain(|&symbol, _| {
            !scoping.symbol_is_mutated(symbol) && only_local_access(program, scoping, &redirectable, symbol)
        });
    }

    fn scan_stmt(&mut self, stmt: &Statement<'_>, scoping: &Scoping) {
        match stmt {
            Statement::VariableDeclaration(decl) => {
                for declarator in &decl.declarations {
                    if let BindingPattern::BindingIdentifier(id) = &declarator.id
                        && let Some(Expression::FunctionExpression(func)) = &declarator.init
                    {
                        self.add(id.symbol_id.get(), func, scoping);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                for inner in iife_body(&es.expression) {
                    self.scan_stmt(inner, scoping);
                }
            }
            Statement::FunctionDeclaration(func) => {
                self.add(func.id.as_ref().and_then(|id| id.symbol_id.get()), func, scoping);
            }
            _ => {}
        }
    }

    fn add(&mut self, symbol: Option<SymbolId>, func: &Function<'_>, scoping: &Scoping) {
        if let Some(symbol) = symbol
            && let Some(op) = Self::extract_equality_op(func, scoping)
        {
            deob_trace!(
                "[AKAMAI/eq-proxy] found  {}(a, b) {{ return a {op:?} b }}",
                scoping.symbol_name(symbol)
            );
            self.proxies.insert(symbol, op);
        }
    }

    fn extract_equality_op(func: &Function<'_>, scoping: &Scoping) -> Option<BinaryOperator> {
        if func.r#async || func.generator || func.params.items.len() != 2 {
            return None;
        }
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };
        let Some(Expression::BinaryExpression(binary)) = &ret.argument else {
            return None;
        };
        // Operator must be a comparison.
        if !matches!(
            binary.operator,
            BinaryOperator::StrictEquality
                | BinaryOperator::StrictInequality
                | BinaryOperator::Equality
                | BinaryOperator::Inequality
                | BinaryOperator::LessThan
                | BinaryOperator::LessEqualThan
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterEqualThan
        ) {
            return None;
        }
        // The operands must read the first and the second parameter, two
        // distinct bindings (sloppy mode allows `function (a, a)`).
        let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
            return None;
        };
        let (Expression::Identifier(l), Expression::Identifier(r)) = (&binary.left, &binary.right) else {
            return None;
        };
        let (p1, p2) = (p1.symbol_id.get()?, p2.symbol_id.get()?);
        (p1 != p2 && resolve(l, scoping) == Some(p1) && resolve(r, scoping) == Some(p2)).then_some(binary.operator)
    }
}

/// The statements of `(function(){...})()` — BMP wraps everything in a paren-IIFE.
fn iife_body<'b, 'a>(expr: &'b Expression<'a>) -> &'b [Statement<'a>] {
    let Expression::CallExpression(call) = expr else {
        return &[];
    };
    let callee = match &call.callee {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    match callee {
        Expression::FunctionExpression(func) => func.body.as_ref().map_or(&[], |body| &body.statements),
        _ => &[],
    }
}

fn resolve(ident: &IdentifierReference<'_>, scoping: &Scoping) -> Option<SymbolId> {
    scoping.get_reference(ident.reference_id()).symbol_id()
}

impl Default for EqualityProxyUnwrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for EqualityProxyUnwrapper {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else { return };
        if call.arguments.len() != 2 || call.arguments.iter().any(Argument::is_spread) {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let Some(&op) = resolve(callee, ctx.scoping()).and_then(|symbol| self.proxies.get(&symbol)) else {
            return;
        };
        if self.unwrapped < 10 {
            deob_trace!(
                "[AKAMAI/eq-proxy] inline {}(a, b) -> a {:?} b  (call #{})",
                callee.name,
                op,
                self.unwrapped + 1
            );
        }
        let left = call.arguments[0]
            .to_expression()
            .clone_in_with_semantic_ids(ctx.ast.allocator());
        let right = call.arguments[1]
            .to_expression()
            .clone_in_with_semantic_ids(ctx.ast.allocator());
        self.unwrapped += 1;
        *expr = Expression::new_binary_expression(SPAN, left, op, right, &ctx.ast);
    }
}
