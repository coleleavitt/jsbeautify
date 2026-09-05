use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    Argument, BinaryExpression, BinaryOperator, BindingPattern, Expression, Function, Program, Statement,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

/// Collects top-level `var NAME = function(a, b) { return a <op> b; };` where
/// `<op>` is one of `===`, `!==`, `==`, `!=`, `<`, `>`, `<=`, `>=`.
///
/// The existing [`OperatorProxyInliner`] handles arithmetic operator proxies
/// but the BMP bundle uses `BI`, `c8`, `mx`, `ZZ`, `Tt`, `QZ`, `WG` etc. for
/// *comparison* operators which `OperatorProxyInliner` does pick up — however
/// that pass inlines every call as a standalone statement. Comparisons show up
/// inside `if`, `?:`, and `&&` where inlining is safe. This pass is a
/// lighter-weight comparison-only variant that focuses on the BMP-specific
/// `BI(typeof x, "undefined")` idiom and unwraps it to `typeof x === "undefined"`.
pub struct EqualityProxyUnwrapper {
    proxies: FxHashMap<String, BinaryOperator>,
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
    pub const fn proxies(&self) -> &FxHashMap<String, BinaryOperator> {
        &self.proxies
    }

    /// Scan the program for equality-proxy definitions. Call before traversal.
    pub fn collect(&mut self, program: &Program<'_>) {
        for stmt in &program.body {
            self.scan_stmt(stmt);
        }
    }

    fn scan_stmt(&mut self, stmt: &Statement<'_>) {
        match stmt {
            Statement::VariableDeclaration(decl) => {
                for declarator in &decl.declarations {
                    self.scan_declarator(declarator);
                }
            }
            Statement::ExpressionStatement(es) => {
                if let Expression::CallExpression(call) = &es.expression {
                    // Unwrap `(function(){...})()` — BMP wraps everything in a paren-IIFE.
                    let callee = match &call.callee {
                        Expression::ParenthesizedExpression(p) => &p.expression,
                        other => other,
                    };
                    if let Expression::FunctionExpression(func) = callee
                        && let Some(body) = func.body.as_ref()
                    {
                        for inner in &body.statements {
                            self.scan_stmt(inner);
                        }
                    }
                }
            }
            Statement::FunctionDeclaration(func) => {
                if let Some((name, op)) = Self::try_extract_equality_proxy_fn(func) {
                    self.proxies.insert(name, op);
                }
            }
            _ => {}
        }
    }

    fn scan_declarator(&mut self, declarator: &VariableDeclarator<'_>) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let Some(Expression::FunctionExpression(func)) = &declarator.init else {
            return;
        };
        if let Some(op) = Self::extract_equality_op(func) {
            let name = id.name.as_str().to_string();
            eprintln!("[AKAMAI/eq-proxy] found  {name}(a, b) {{ return a {op:?} b }}");
            self.proxies.insert(name, op);
        }
    }

    fn try_extract_equality_proxy_fn(func: &Function<'_>) -> Option<(String, BinaryOperator)> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        let op = Self::extract_equality_op(func)?;
        eprintln!("[AKAMAI/eq-proxy] found  function {name}(a, b) {{ return a {op:?} b }}");
        Some((name, op))
    }

    fn extract_equality_op(func: &Function<'_>) -> Option<BinaryOperator> {
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
        // Both sides must be identifiers that match the parameters.
        let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
            return None;
        };
        let Expression::Identifier(l) = &binary.left else {
            return None;
        };
        let Expression::Identifier(r) = &binary.right else {
            return None;
        };
        if l.name.as_str() != p1.name.as_str() || r.name.as_str() != p2.name.as_str() {
            return None;
        }
        Some(binary.operator)
    }
}

impl Default for EqualityProxyUnwrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for EqualityProxyUnwrapper {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else { return };
        if call.arguments.len() != 2 {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let name = callee.name.as_str();
        let Some(&op) = self.proxies.get(name) else { return };

        // Only inline when both arguments are simple expressions (no spread).
        let Argument::SpreadElement(_) = &call.arguments[0] else {
            let Argument::SpreadElement(_) = &call.arguments[1] else {
                let left = call.arguments[0]
                    .to_expression()
                    .clone_in_with_semantic_ids(ctx.allocator());
                let right = call.arguments[1]
                    .to_expression()
                    .clone_in_with_semantic_ids(ctx.allocator());
                if self.unwrapped < 10 {
                    eprintln!(
                        "[AKAMAI/eq-proxy] inline {}(a, b) -> a {:?} b  (call #{})",
                        name,
                        op,
                        self.unwrapped + 1
                    );
                }
                self.unwrapped += 1;
                *expr = Expression::BinaryExpression(ctx.ast.alloc(BinaryExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    left,
                    operator: op,
                    right,
                }));
                return;
            };
            return;
        };
    }
}
