use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{CallExpression, Expression, Statement};
use rustc_hash::FxHashMap;

use super::super::CaseBodyMap;
use super::DispatcherMeta;
use crate::ast_deobfuscate::dispatcher_detector::DispatcherMap;

pub(super) type CallResolution<'a, 'b> = (
    &'b str,
    &'b str,
    &'b Expression<'a>,
    &'b DispatcherMeta,
    &'b OxcVec<'a, Statement<'a>>,
);

fn resolve_direct<'a, 'b>(
    dispatchers: &'b DispatcherMap,
    case_bodies: &'b CaseBodyMap<'a>,
    meta_map: &'b FxHashMap<String, DispatcherMeta>,
    call: &'b CallExpression<'a>,
) -> Option<CallResolution<'a, 'b>> {
    let Expression::Identifier(callee_id) = &call.callee else {
        return None;
    };
    let name = callee_id.name.as_str();
    if !dispatchers.contains_key(name) {
        return None;
    }
    if call.arguments.len() != 2 {
        return None;
    }
    let state_expr = call.arguments[0].as_expression()?;
    let Expression::Identifier(state_id) = state_expr else {
        return None;
    };
    let state_label = state_id.name.as_str();
    let key = (name.to_string(), state_label.to_string());
    if !case_bodies.contains_key(&key) {
        return None;
    }
    let args_expr = call.arguments[1].as_expression()?;
    let meta = meta_map.get(name)?;
    let body = case_bodies.get(&key)?;
    Some((name, state_label, args_expr, meta, body))
}

fn resolve_call_this<'a, 'b>(
    dispatchers: &'b DispatcherMap,
    case_bodies: &'b CaseBodyMap<'a>,
    meta_map: &'b FxHashMap<String, DispatcherMeta>,
    call: &'b CallExpression<'a>,
) -> Option<CallResolution<'a, 'b>> {
    let Expression::StaticMemberExpression(sme) = &call.callee else {
        return None;
    };
    if sme.property.name.as_str() != "call" {
        return None;
    }
    let Expression::Identifier(obj_id) = &sme.object else {
        return None;
    };
    let name = obj_id.name.as_str();
    if !dispatchers.contains_key(name) {
        return None;
    }
    if call.arguments.len() != 3 {
        return None;
    }
    let first = call.arguments[0].as_expression()?;
    if !matches!(first, Expression::ThisExpression(_)) {
        return None;
    }
    let state_expr = call.arguments[1].as_expression()?;
    let Expression::Identifier(state_id) = state_expr else {
        return None;
    };
    let state_label = state_id.name.as_str();
    let key = (name.to_string(), state_label.to_string());
    if !case_bodies.contains_key(&key) {
        return None;
    }
    let args_expr = call.arguments[2].as_expression()?;
    let meta = meta_map.get(name)?;
    let body = case_bodies.get(&key)?;
    Some((name, state_label, args_expr, meta, body))
}

pub(super) fn resolve_call<'a, 'b>(
    dispatchers: &'b DispatcherMap,
    case_bodies: &'b CaseBodyMap<'a>,
    meta_map: &'b FxHashMap<String, DispatcherMeta>,
    call: &'b CallExpression<'a>,
) -> Option<CallResolution<'a, 'b>> {
    resolve_direct(dispatchers, case_bodies, meta_map, call)
        .or_else(|| resolve_call_this(dispatchers, case_bodies, meta_map, call))
}
