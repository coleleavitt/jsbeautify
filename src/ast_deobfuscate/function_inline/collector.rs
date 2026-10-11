use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, CallExpression, Expression, Function, FunctionBody, Program, Statement,
    UnaryOperator,
};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{BinaryOp, InlinableFunction, MAX_PARAMS, ReturnExpr, UnaryOp};

/// The call through a function's only reference.
struct CallSite {
    scope_id: ScopeId,
    arg_count: usize,
    pure_args: bool,
}

/// Finds function declarations whose single reference is a call that can be
/// replaced by the function's returned expression.
///
/// A candidate is a plain `function f(a, b) { return <expr>; }` declaration
/// where `<expr>` is built from literals, parameters and free identifiers with
/// side-effect-free operators. Its only reference must be a call `f(x, y)`
/// whose arguments are literals or reads of never-reassigned bindings, so they
/// can be duplicated, dropped or reordered, and where every free identifier of
/// `<expr>` resolves to the same binding as inside `f`.
pub struct FunctionCollector {
    candidates: FxHashMap<SymbolId, InlinableFunction>,
    call_sites: FxHashMap<SymbolId, CallSite>,
    inlinable: FxHashMap<SymbolId, InlinableFunction>,
}

impl FunctionCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: FxHashMap::default(),
            call_sites: FxHashMap::default(),
            inlinable: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn get_single_use_functions(&self) -> FxHashMap<SymbolId, InlinableFunction> {
        self.inlinable.clone()
    }

    fn try_extract_inlinable(func: &Function<'_>, scoping: &Scoping) -> Option<InlinableFunction> {
        let name = func.id.as_ref()?.name.as_str().to_string();
        if func.r#async || func.generator || func.params.rest.is_some() || func.params.items.len() > MAX_PARAMS {
            return None;
        }
        let params = func
            .params
            .items
            .iter()
            .map(|param| match &param.pattern {
                BindingPattern::BindingIdentifier(ident) if param.initializer.is_none() => ident.symbol_id.get(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let body = func.body.as_ref()?;
        let return_expression = Self::extract_return_expression(body, &params, scoping)?;
        // Dropping an argument would also drop its temporal-dead-zone check.
        if !(0..params.len()).all(|index| return_expression.uses_param(index)) {
            return None;
        }
        Some(InlinableFunction {
            name,
            params: params.len(),
            return_expression,
        })
    }

    fn extract_return_expression(
        body: &FunctionBody<'_>,
        params: &[SymbolId],
        scoping: &Scoping,
    ) -> Option<ReturnExpr> {
        if !body.directives.is_empty() {
            return None;
        }
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return None;
        };
        Self::expr_to_return_expr(ret.argument.as_ref()?, params, scoping)
    }

    fn expr_to_return_expr(expr: &Expression<'_>, params: &[SymbolId], scoping: &Scoping) -> Option<ReturnExpr> {
        match expr {
            Expression::Identifier(ident) => {
                let symbol = scoping.get_reference(ident.reference_id()).symbol_id();
                if let Some(index) = symbol.and_then(|symbol| params.iter().position(|&param| param == symbol)) {
                    return Some(ReturnExpr::ParamRef(index));
                }
                // `arguments` names the function's own arguments object.
                (ident.name != "arguments").then(|| ReturnExpr::Identifier {
                    name: ident.name.to_string(),
                    symbol,
                })
            }
            Expression::NumericLiteral(num) => Some(ReturnExpr::Number(num.value)),
            Expression::StringLiteral(s) => Some(ReturnExpr::String(s.value.as_str().to_string())),
            Expression::BooleanLiteral(b) => Some(ReturnExpr::Boolean(b.value)),
            Expression::NullLiteral(_) => Some(ReturnExpr::Null),
            Expression::BinaryExpression(binary) => {
                let left = Self::expr_to_return_expr(&binary.left, params, scoping)?;
                let right = Self::expr_to_return_expr(&binary.right, params, scoping)?;
                let op = match binary.operator {
                    BinaryOperator::Addition => BinaryOp::Add,
                    BinaryOperator::Subtraction => BinaryOp::Sub,
                    BinaryOperator::Multiplication => BinaryOp::Mul,
                    BinaryOperator::Division => BinaryOp::Div,
                    BinaryOperator::Remainder => BinaryOp::Mod,
                    BinaryOperator::BitwiseAnd => BinaryOp::BitAnd,
                    BinaryOperator::BitwiseOR => BinaryOp::BitOr,
                    BinaryOperator::BitwiseXOR => BinaryOp::BitXor,
                    BinaryOperator::Equality => BinaryOp::Eq,
                    BinaryOperator::Inequality => BinaryOp::NotEq,
                    BinaryOperator::StrictEquality => BinaryOp::StrictEq,
                    BinaryOperator::StrictInequality => BinaryOp::StrictNotEq,
                    BinaryOperator::LessThan => BinaryOp::Lt,
                    BinaryOperator::LessEqualThan => BinaryOp::Lte,
                    BinaryOperator::GreaterThan => BinaryOp::Gt,
                    BinaryOperator::GreaterEqualThan => BinaryOp::Gte,
                    _ => return None,
                };
                Some(ReturnExpr::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                })
            }
            Expression::UnaryExpression(unary) => {
                let arg = Self::expr_to_return_expr(&unary.argument, params, scoping)?;
                let op = match unary.operator {
                    UnaryOperator::LogicalNot => UnaryOp::Not,
                    UnaryOperator::UnaryNegation => UnaryOp::Neg,
                    UnaryOperator::UnaryPlus => UnaryOp::Plus,
                    UnaryOperator::BitwiseNot => UnaryOp::BitNot,
                    _ => return None,
                };
                Some(ReturnExpr::Unary { op, arg: Box::new(arg) })
            }
            Expression::ParenthesizedExpression(paren) => Self::expr_to_return_expr(&paren.expression, params, scoping),
            _ => None,
        }
    }

    fn is_pure_argument(arg: &Argument<'_>, scoping: &Scoping) -> bool {
        match arg {
            Argument::NumericLiteral(_)
            | Argument::StringLiteral(_)
            | Argument::BooleanLiteral(_)
            | Argument::NullLiteral(_) => true,
            Argument::Identifier(ident) => scoping
                .get_reference(ident.reference_id())
                .symbol_id()
                .is_some_and(|symbol| !scoping.symbol_is_mutated(symbol)),
            _ => false,
        }
    }

    fn free_identifiers_resolve(expr: &ReturnExpr, scope_id: ScopeId, scoping: &Scoping) -> bool {
        match expr {
            ReturnExpr::Identifier { name, symbol } => scoping.find_binding(scope_id, name.as_str().into()) == *symbol,
            ReturnExpr::Binary { left, right, .. } => {
                Self::free_identifiers_resolve(left, scope_id, scoping)
                    && Self::free_identifiers_resolve(right, scope_id, scoping)
            }
            ReturnExpr::Unary { arg, .. } => Self::free_identifiers_resolve(arg, scope_id, scoping),
            ReturnExpr::Number(_)
            | ReturnExpr::String(_)
            | ReturnExpr::Boolean(_)
            | ReturnExpr::Null
            | ReturnExpr::ParamRef(_) => true,
        }
    }

    fn is_inlinable(&self, symbol: SymbolId, func: &InlinableFunction, scoping: &Scoping) -> bool {
        let Some(call) = self.call_sites.get(&symbol) else {
            return false;
        };
        scoping.get_resolved_reference_ids(symbol).len() == 1
            && call.pure_args
            && call.arg_count == func.params
            && Self::free_identifiers_resolve(&func.return_expression, call.scope_id, scoping)
    }
}

impl Default for FunctionCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for FunctionCollector {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func) = stmt
            && let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get())
            && let Some(inlinable) = Self::try_extract_inlinable(func, ctx.scoping())
        {
            self.candidates.insert(symbol, inlinable);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::Identifier(ident) = &call.callee else {
            return;
        };
        let Some(symbol) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        let pure_args = call
            .arguments
            .iter()
            .all(|arg| Self::is_pure_argument(arg, ctx.scoping()));
        self.call_sites.insert(
            symbol,
            CallSite {
                scope_id: ctx.current_scope_id(),
                arg_count: call.arguments.len(),
                pure_args,
            },
        );
    }

    fn exit_program(&mut self, _program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let candidates = std::mem::take(&mut self.candidates);
        for (symbol, func) in candidates {
            if self.is_inlinable(symbol, &func, ctx.scoping()) {
                eprintln!("[AST] Found inlinable function: {} ({} params)", func.name, func.params);
                self.inlinable.insert(symbol, func);
            }
        }
    }
}
