use oxc_allocator::{ArenaVec, TakeIn};
use oxc_ast::ast::{BinaryOperator, Expression, TemplateElement, TemplateLiteral};

use super::literal::{make_string_literal, make_template_element};
use super::{Ctx, DynConcatResolver, MAX_INLINE_LEN};

/// The string value of a literal, tracked binding or `+` chain of them, when
/// it is at most `MAX_INLINE_LEN` bytes long.
pub(super) fn try_resolve_to_string(
    expr: &Expression<'_>,
    resolver: &DynConcatResolver,
    ctx: &Ctx<'_>,
) -> Option<String> {
    let mut value = String::new();
    append_resolved(expr, resolver, ctx, &mut value).then_some(value)
}

/// Appends `expr`'s value to `out`, stopping as soon as the value is known or
/// grows past `MAX_INLINE_LEN`, so a long chain costs linear time.
fn append_resolved(expr: &Expression<'_>, resolver: &DynConcatResolver, ctx: &Ctx<'_>, out: &mut String) -> bool {
    match expr {
        Expression::StringLiteral(lit) if !lit.lone_surrogates => out.push_str(&lit.value),
        Expression::ParenthesizedExpression(p) => return append_resolved(&p.expression, resolver, ctx, out),
        Expression::Identifier(id) => {
            let Some(value) = ctx
                .scoping()
                .get_reference(id.reference_id())
                .symbol_id()
                .and_then(|symbol_id| resolver.lookup_ident(symbol_id, ctx))
            else {
                return false;
            };
            out.push_str(value);
        }
        Expression::BinaryExpression(b) if matches!(b.operator, BinaryOperator::Addition) => {
            return append_resolved(&b.left, resolver, ctx, out) && append_resolved(&b.right, resolver, ctx, out);
        }
        _ => return false,
    }
    out.len() <= MAX_INLINE_LEN
}

pub(super) fn chain_has_string_evidence(expr: &Expression<'_>, resolver: &DynConcatResolver, ctx: &Ctx<'_>) -> bool {
    match expr {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => true,
        Expression::ParenthesizedExpression(p) => chain_has_string_evidence(&p.expression, resolver, ctx),
        Expression::Identifier(id) => ctx
            .scoping()
            .get_reference(id.reference_id())
            .symbol_id()
            .and_then(|sid| resolver.lookup_ident(sid, ctx))
            .is_some(),
        Expression::BinaryExpression(b) if matches!(b.operator, BinaryOperator::Addition) => {
            chain_has_string_evidence(&b.left, resolver, ctx) || chain_has_string_evidence(&b.right, resolver, ctx)
        }
        _ => false,
    }
}

pub(super) fn substitute_in_chain<'a>(expr: &mut Expression<'a>, resolver: &DynConcatResolver, ctx: &Ctx<'a>) -> usize {
    let mut count = 0;
    match expr {
        Expression::ParenthesizedExpression(p) => {
            count += substitute_in_chain(&mut p.expression, resolver, ctx);
        }
        Expression::BinaryExpression(b) if matches!(b.operator, BinaryOperator::Addition) => {
            count += substitute_in_chain(&mut b.left, resolver, ctx);
            count += substitute_in_chain(&mut b.right, resolver, ctx);
        }
        Expression::Identifier(id) => {
            if let Some(symbol_id) = ctx.scoping().get_reference(id.reference_id()).symbol_id()
                && let Some(value) = resolver.lookup_ident(symbol_id, ctx)
            {
                let val = value.to_string();
                *expr = make_string_literal(&val, ctx);
                count += 1;
            }
        }
        _ => {}
    }
    count
}

/// Folds every `<string> + <string>` in a `+` chain, including chains nested
/// in parentheses, which the traversal leaves to the outermost chain.
pub(super) fn fold_adjacent_literals<'a>(expr: &mut Expression<'a>, ctx: &Ctx<'a>) -> bool {
    if let Expression::ParenthesizedExpression(p) = expr {
        return fold_adjacent_literals(&mut p.expression, ctx);
    }
    let Expression::BinaryExpression(b) = expr else {
        return false;
    };
    if !matches!(b.operator, BinaryOperator::Addition) {
        return false;
    }
    let mut changed = fold_adjacent_literals(&mut b.left, ctx);
    changed |= fold_adjacent_literals(&mut b.right, ctx);

    if let (Expression::StringLiteral(l), Expression::StringLiteral(r)) = (&b.left, &b.right)
        && !l.lone_surrogates
        && !r.lone_surrogates
    {
        let combined = format!("{}{}", l.value.as_str(), r.value.as_str());
        if combined.len() <= MAX_INLINE_LEN {
            *expr = make_string_literal(&combined, ctx);
            return true;
        }
    }
    changed
}

pub(super) fn substitute_in_template<'a>(
    tmpl: &mut TemplateLiteral<'a>,
    resolver: &DynConcatResolver,
    ctx: &Ctx<'a>,
) -> usize {
    if tmpl
        .quasis
        .iter()
        .any(|quasi| quasi.lone_surrogates || quasi.value.cooked.is_none())
    {
        return 0;
    }
    let values: Vec<Option<&str>> = tmpl
        .expressions
        .iter()
        .map(|interp| match interp {
            Expression::Identifier(id) => ctx
                .scoping()
                .get_reference(id.reference_id())
                .symbol_id()
                .and_then(|sid| resolver.lookup_ident(sid, ctx)),
            _ => None,
        })
        .collect();
    let count = values.iter().flatten().count();
    if count == 0 {
        return 0;
    }

    // Kept interpolations move into the rebuilt template: cloning them would
    // copy every nested template once per enclosing level.
    let expressions = tmpl.expressions.take_in(ctx);
    let mut new_quasis: ArenaVec<'a, TemplateElement<'a>> = ArenaVec::with_capacity_in(tmpl.quasis.len() - count, ctx);
    let mut kept_expressions = ArenaVec::with_capacity_in(expressions.len() - count, ctx);
    let mut pending_text = String::new();
    for ((quasi, interp), value) in tmpl.quasis.iter().zip(expressions).zip(values) {
        pending_text.push_str(quasi.value.cooked.as_ref().expect("checked above").as_str());
        if let Some(value) = value {
            pending_text.push_str(value);
        } else {
            new_quasis.push(make_template_element(&pending_text, false, ctx));
            kept_expressions.push(interp);
            pending_text.clear();
        }
    }
    let tail = tmpl
        .quasis
        .last()
        .and_then(|quasi| quasi.value.cooked.as_ref())
        .expect("checked above");
    pending_text.push_str(tail.as_str());
    new_quasis.push(make_template_element(&pending_text, true, ctx));

    tmpl.quasis = new_quasis;
    tmpl.expressions = kept_expressions;
    count
}
