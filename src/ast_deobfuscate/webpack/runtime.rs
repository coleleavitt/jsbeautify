use oxc_ast::ast::{
    Argument, AssignmentExpression, AssignmentTarget, CallExpression, Expression, Function, ObjectPropertyKind,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Runtime;
use super::shapes::{Ctx, chunk_payload, resolve_expr, simple_params};

/// Finds webpack bootstraps: a named single-parameter function `R(id)` that
/// invokes a factory as `M[id](m, m.exports, R)` or
/// `M[id].call(m.exports, m, m.exports, R)`. Also notes whether any chunk
/// payload is present.
#[derive(Default)]
pub(super) struct RuntimeCollector {
    /// `(function symbol, module-id parameter)` per enclosing non-arrow function.
    frames: Vec<Option<(SymbolId, SymbolId)>>,
    runtimes: Vec<Runtime>,
    caches: Vec<(SymbolId, SymbolId)>,
    has_chunk: bool,
}

impl RuntimeCollector {
    pub(super) const fn has_chunk(&self) -> bool {
        self.has_chunk
    }

    pub(super) fn into_runtimes(mut self) -> Vec<Runtime> {
        for runtime in &mut self.runtimes {
            runtime.cache = self
                .caches
                .iter()
                .find(|(require, cache)| *require == runtime.require && *cache != runtime.modules)
                .map(|&(_, cache)| cache);
        }
        self.runtimes
    }

    fn frame(&self) -> Option<(SymbolId, SymbolId)> {
        self.frames.last().copied().flatten()
    }
}

/// `m.exports` where `m` resolves to `module`.
fn is_exports_of(expr: &Expression<'_>, module: SymbolId, ctx: &Ctx<'_>) -> bool {
    matches!(expr, Expression::StaticMemberExpression(member)
        if member.property.name == "exports" && resolve_expr(&member.object, ctx) == Some(module))
}

fn arg_expr<'b, 'a>(args: &'b [Argument<'a>], i: usize) -> Option<&'b Expression<'a>> {
    args.get(i)?.as_expression()
}

/// `(modules, key, require)` symbols of a factory invocation.
fn factory_invocation(call: &CallExpression<'_>, ctx: &Ctx<'_>) -> Option<(SymbolId, SymbolId, SymbolId)> {
    let (lookup, args) = match &call.callee {
        Expression::ComputedMemberExpression(lookup) if call.arguments.len() == 3 => {
            (lookup, call.arguments.as_slice())
        }
        Expression::StaticMemberExpression(member) if member.property.name == "call" && call.arguments.len() == 4 => {
            let Expression::ComputedMemberExpression(lookup) = &member.object else {
                return None;
            };
            let module = resolve_expr(arg_expr(&call.arguments, 1)?, ctx)?;
            if !is_exports_of(arg_expr(&call.arguments, 0)?, module, ctx) {
                return None;
            }
            (lookup, &call.arguments[1..])
        }
        _ => return None,
    };
    let module = resolve_expr(arg_expr(args, 0)?, ctx)?;
    if !is_exports_of(arg_expr(args, 1)?, module, ctx) {
        return None;
    }
    let require = resolve_expr(arg_expr(args, 2)?, ctx)?;
    Some((
        resolve_expr(&lookup.object, ctx)?,
        resolve_expr(&lookup.expression, ctx)?,
        require,
    ))
}

impl<'a> Traverse<'a, DeobfuscateState> for RuntimeCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        let frame = func.id.as_ref().and_then(|id| id.symbol_id.get()).and_then(|f| {
            match simple_params(&func.params)?.as_slice() {
                [module_id] => Some((f, *module_id)),
                _ => None,
            }
        });
        self.frames.push(frame);
    }

    fn exit_function(&mut self, _func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        self.frames.pop();
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        if chunk_payload(call).is_some() {
            self.has_chunk = true;
        }
        let Some((require, module_id)) = self.frame() else {
            return;
        };
        if let Some((modules, key, callee_require)) = factory_invocation(call, ctx)
            && key == module_id
            && callee_require == require
            && modules != require
            && !self.runtimes.iter().any(|r| r.require == require)
        {
            self.runtimes.push(Runtime {
                require,
                module_id,
                modules,
                cache: None,
            });
        }
    }

    /// `C[id] = { …, exports: {} }` inside a candidate require function.
    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        let Some((require, module_id)) = self.frame() else {
            return;
        };
        let AssignmentTarget::ComputedMemberExpression(slot) = &assign.left else {
            return;
        };
        let Expression::ObjectExpression(record) = &assign.right else {
            return;
        };
        let has_exports = record.properties.iter().any(
            |prop| matches!(prop, ObjectPropertyKind::ObjectProperty(p) if p.key.is_specific_static_name("exports")),
        );
        if has_exports
            && resolve_expr(&slot.expression, ctx) == Some(module_id)
            && let Some(cache) = resolve_expr(&slot.object, ctx)
        {
            self.caches.push((require, cache));
        }
    }
}
