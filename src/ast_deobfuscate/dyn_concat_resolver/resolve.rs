use oxc_allocator::CloneIn;
use oxc_ast::ast::{BinaryOperator, Expression, TemplateElement, TemplateLiteral};

use super::literal::{make_string_literal, make_template_element};
use super::member::member_key_from_static_member;
use super::{Ctx, DynConcatResolver, MAX_INLINE_LEN};

pub(super) fn try_resolve_to_string(
    expr: &Expression<'_>,
    resolver: &DynConcatResolver,
    ctx: &Ctx<'_>,
) -> Option<String> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.as_str().to_string()),
        Expression::ParenthesizedExpression(p) => try_resolve_to_string(&p.expression, resolver, ctx),
        Expression::Identifier(id) => {
            let symbol_id = ctx.scoping().get_reference(id.reference_id()).symbol_id()?;
            resolver.lookup_ident(symbol_id, ctx).map(str::to_string)
        }
        Expression::StaticMemberExpression(sme) => {
            let key = member_key_from_static_member(&sme.object, sme.property.name.as_str())?;
            resolver.lookup_member(&key).map(str::to_string)
        }
        Expression::BinaryExpression(b) if matches!(b.operator, BinaryOperator::Addition) => {
            let l = try_resolve_to_string(&b.left, resolver, ctx)?;
            let r = try_resolve_to_string(&b.right, resolver, ctx)?;
            Some(l + &r)
        }
        _ => None,
    }
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
        Expression::StaticMemberExpression(sme) => {
            member_key_from_static_member(&sme.object, sme.property.name.as_str())
                .and_then(|key| resolver.lookup_member(&key))
                .is_some()
        }
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
        Expression::StaticMemberExpression(sme) => {
            if let Some(key) = member_key_from_static_member(&sme.object, sme.property.name.as_str())
                && let Some(value) = resolver.lookup_member(&key)
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

pub(super) fn fold_adjacent_literals<'a>(expr: &mut Expression<'a>, ctx: &Ctx<'a>) -> bool {
    let Expression::BinaryExpression(b) = expr else {
        return false;
    };
    if !matches!(b.operator, BinaryOperator::Addition) {
        return false;
    }
    let mut changed = fold_adjacent_literals(&mut b.left, ctx);
    changed |= fold_adjacent_literals(&mut b.right, ctx);

    if let (Expression::StringLiteral(l), Expression::StringLiteral(r)) = (&b.left, &b.right) {
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
    let mut count = 0;
    let mut new_quasis: Vec<TemplateElement<'a>> = Vec::with_capacity(tmpl.quasis.len());
    let mut rebuilt_expressions: Vec<Expression<'a>> = Vec::new();
    let mut pending_text = String::new();

    for (i, quasi) in tmpl.quasis.iter().enumerate() {
        pending_text.push_str(quasi.value.cooked.as_ref().unwrap_or(&quasi.value.raw).as_str());
        if i < tmpl.expressions.len() {
            let interp = &tmpl.expressions[i];
            let replacement_value = match interp {
                Expression::Identifier(id) => ctx
                    .scoping()
                    .get_reference(id.reference_id())
                    .symbol_id()
                    .and_then(|sid| resolver.lookup_ident(sid, ctx))
                    .map(str::to_string),
                Expression::StaticMemberExpression(sme) => {
                    member_key_from_static_member(&sme.object, sme.property.name.as_str())
                        .and_then(|k| resolver.lookup_member(&k))
                        .map(str::to_string)
                }
                _ => None,
            };
            if let Some(value) = replacement_value {
                pending_text.push_str(&value);
                count += 1;
                continue;
            }
            new_quasis.push(make_template_element(&pending_text, false, ctx));
            rebuilt_expressions.push(interp.clone_in_with_semantic_ids(ctx.ast.allocator));
            pending_text.clear();
        }
    }

    new_quasis.push(make_template_element(&pending_text, true, ctx));

    if count > 0 {
        tmpl.quasis = ctx.ast.vec_from_iter(new_quasis);
        tmpl.expressions = ctx.ast.vec_from_iter(rebuilt_expressions);
    }
    count
}
