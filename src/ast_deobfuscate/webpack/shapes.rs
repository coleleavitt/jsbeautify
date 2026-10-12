use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, CallExpression, Expression, FormalParameters,
    IdentifierReference, ObjectPropertyKind, PropertyKey, Statement,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub(super) type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub fn resolve(id: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> Option<SymbolId> {
    ctx.scoping().get_reference(id.reference_id()).symbol_id()
}

pub fn resolve_expr(expr: &Expression<'_>, ctx: &Ctx<'_>) -> Option<SymbolId> {
    match expr.without_parentheses() {
        Expression::Identifier(id) => resolve(id, ctx),
        _ => None,
    }
}

/// Symbols of `params` when every parameter is a plain identifier without a
/// default value or rest element.
pub fn simple_params(params: &FormalParameters<'_>) -> Option<Vec<SymbolId>> {
    if params.rest.is_some() {
        return None;
    }
    params
        .items
        .iter()
        .map(|param| match &param.pattern {
            BindingPattern::BindingIdentifier(id) if param.initializer.is_none() => id.symbol_id.get(),
            _ => None,
        })
        .collect()
}

/// Parameters of a plain (non-async, non-generator) function or arrow.
pub(super) fn function_params<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b FormalParameters<'a>> {
    match expr.without_parentheses() {
        Expression::FunctionExpression(func) if !func.r#async && !func.generator => Some(&func.params),
        Expression::ArrowFunctionExpression(arrow) if !arrow.r#async => Some(&arrow.params),
        _ => None,
    }
}

fn module_id(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// The module ids and factory functions of a webpack module collection: an
/// object keyed by module id (`{12: function(e,t,n){…}, "./a.js"(e){…}}`) or a
/// factory array indexed by id (`[function(e,t){…}, , function(e){…}]`).
/// Every entry must be a factory.
pub(super) fn module_factories<'b, 'a>(expr: &'b Expression<'a>) -> Option<Vec<(String, &'b FormalParameters<'a>)>> {
    match expr.without_parentheses() {
        Expression::ObjectExpression(obj) => obj
            .properties
            .iter()
            .map(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) if !p.computed => {
                    Some((module_id(&p.key)?, function_params(&p.value)?))
                }
                _ => None,
            })
            .collect(),
        Expression::ArrayExpression(arr) => arr
            .elements
            .iter()
            .enumerate()
            .filter(|(_, el)| !matches!(el, ArrayExpressionElement::Elision(_)))
            .map(|(id, el)| Some((id.to_string(), el.as_expression().and_then(function_params)?)))
            .collect(),
        _ => None,
    }
}

fn is_chunk_id_list(el: Option<&ArrayExpressionElement<'_>>) -> bool {
    let Some(ArrayExpressionElement::ArrayExpression(ids)) = el else {
        return false;
    };
    ids.elements.iter().all(|id| {
        matches!(
            id,
            ArrayExpressionElement::NumericLiteral(_) | ArrayExpressionElement::StringLiteral(_)
        )
    })
}

fn is_chunk_callback_name(name: &str) -> bool {
    name.starts_with("webpackJsonp") || name.starts_with("webpackChunk")
}

/// A webpack chunk payload: `chunks.push([[ids], modules, runtime?])`
/// (webpack 4/5 JSONP and array-push formats) or the legacy callback call
/// `webpackJsonp([ids], modules, …)` / `self.webpackChunk([ids], modules)`.
/// Returns the module collection and the optional chunk runtime function.
pub(super) fn chunk_payload<'b, 'a>(
    call: &'b CallExpression<'a>,
) -> Option<(&'b Expression<'a>, Option<&'b Expression<'a>>)> {
    let callee_name = match call.callee.without_parentheses() {
        Expression::Identifier(id) => id.name.as_str(),
        Expression::StaticMemberExpression(member) => member.property.name.as_str(),
        _ => return None,
    };

    if callee_name == "push" {
        let [Argument::ArrayExpression(payload)] = call.arguments.as_slice() else {
            return None;
        };
        if !(2..=3).contains(&payload.elements.len()) || !is_chunk_id_list(payload.elements.first()) {
            return None;
        }
        let modules = payload.elements.get(1)?.as_expression()?;
        module_factories(modules)?;
        let runtime = payload.elements.get(2).and_then(ArrayExpressionElement::as_expression);
        return Some((modules, runtime));
    }

    if !is_chunk_callback_name(callee_name) || call.arguments.len() < 2 {
        return None;
    }
    let Some(Argument::ArrayExpression(ids)) = call.arguments.first() else {
        return None;
    };
    let ids_ok = ids.elements.iter().all(|id| {
        matches!(
            id,
            ArrayExpressionElement::NumericLiteral(_) | ArrayExpressionElement::StringLiteral(_)
        )
    });
    let modules = call.arguments.get(1)?.as_expression()?;
    (ids_ok && module_factories(modules).is_some()).then_some((modules, None))
}

/// The identifier an export getter returns: `() => x` or `function () { return x }`.
pub fn getter_target<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b IdentifierReference<'a>> {
    match expr.without_parentheses() {
        Expression::ArrowFunctionExpression(arrow) if arrow.params.items.is_empty() => {
            match arrow.body.as_expression()?.without_parentheses() {
                Expression::Identifier(id) => Some(id),
                _ => None,
            }
        }
        Expression::FunctionExpression(func) if func.params.items.is_empty() && func.params.rest.is_none() => {
            let [Statement::ReturnStatement(ret)] = func.body.as_ref()?.statements.as_slice() else {
                return None;
            };
            match ret.argument.as_ref()?.without_parentheses() {
                Expression::Identifier(id) => Some(id),
                _ => None,
            }
        }
        _ => None,
    }
}
