use oxc_ast::ast::{CallExpression, Expression, IdentifierReference};
use rustc_hash::FxHashMap;

use super::super::{CaseBody, CaseBodyMap};
use super::DispatcherMeta;
use crate::ast_deobfuscate::dispatcher_detector::DispatcherMap;

/// A call site matched to a dispatcher case.
pub(super) struct CallResolution<'a, 'b> {
    pub(super) callee: &'b IdentifierReference<'a>,
    pub(super) label: &'b IdentifierReference<'a>,
    pub(super) args: &'b Expression<'a>,
    pub(super) meta: &'b DispatcherMeta,
    pub(super) body: &'b CaseBody<'a>,
    /// `D.call(this, …)`: the case body sees the call site's `this`.
    pub(super) forwards_this: bool,
}

fn resolve<'a, 'b>(
    case_bodies: &'b CaseBodyMap<'a>,
    meta_map: &'b FxHashMap<String, DispatcherMeta>,
    callee: &'b IdentifierReference<'a>,
    label: &'b Expression<'a>,
    args: &'b Expression<'a>,
    forwards_this: bool,
) -> Option<CallResolution<'a, 'b>> {
    let Expression::Identifier(label) = label else {
        return None;
    };
    let key = (callee.name.to_string(), label.name.to_string());
    Some(CallResolution {
        callee,
        label,
        args,
        meta: meta_map.get(callee.name.as_str())?,
        body: case_bodies.get(&key)?,
        forwards_this,
    })
}

pub(super) fn resolve_call<'a, 'b>(
    dispatchers: &'b DispatcherMap,
    case_bodies: &'b CaseBodyMap<'a>,
    meta_map: &'b FxHashMap<String, DispatcherMeta>,
    call: &'b CallExpression<'a>,
) -> Option<CallResolution<'a, 'b>> {
    let args: Vec<&Expression<'a>> = call
        .arguments
        .iter()
        .map(|arg| arg.as_expression())
        .collect::<Option<_>>()?;
    match (&call.callee, args.as_slice()) {
        (Expression::Identifier(callee), [label, args]) if dispatchers.contains_key(callee.name.as_str()) => {
            resolve(case_bodies, meta_map, callee, label, args, false)
        }
        (Expression::StaticMemberExpression(sme), [Expression::ThisExpression(_), label, args])
            if sme.property.name == "call" =>
        {
            let Expression::Identifier(callee) = &sme.object else {
                return None;
            };
            if !dispatchers.contains_key(callee.name.as_str()) {
                return None;
            }
            resolve(case_bodies, meta_map, callee, label, args, true)
        }
        _ => None,
    }
}
