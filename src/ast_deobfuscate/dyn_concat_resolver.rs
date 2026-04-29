//! Dynamic-concatenation resolver: inlines string-valued bindings into
//! `+` chains and template-literal interpolations so endpoint extractors see
//! the full URL pattern.
//!
//! Tracked binding kinds:
//!  - `var/let/const NAME = "string"`              → keyed by [`SymbolId`]
//!  - `this.PROP = "string"` (inside any function)  → keyed by [`MemberKey::This`]
//!  - `obj.PROP = "string"` where `obj` is an ident → keyed by [`MemberKey::Bound`]
//!
//! Substitution sites:
//!  - any operand of a `BinaryExpression(+)` (recursive into nested chains)
//!  - any expression of a `TemplateLiteral` interpolation
//!
//! Anti-FP design:
//!  - Identifier bindings: substitute only when [`Scoping::symbol_is_mutated`]
//!    is false (oxc tracks this for free via the symbol table).
//!  - Member-expression bindings: a `poisoned` set tracks any reassignment.
//!  - Numeric `+`: a `+` chain is only treated as string-concat if at least
//!    one operand is a string literal OR a known string-valued binding.
//!
//! Driver loop: this pass is run repeatedly from `mod.rs` (fixed-point) until
//! `state.changed == false`, so chained declarations like `var B = A + "/v1"`
//! resolve in successive iterations once `A` is folded.

use oxc_allocator::CloneIn;
use oxc_ast::ast::{
    AssignmentExpression, AssignmentOperator, AssignmentTarget, BinaryExpression, BinaryOperator, Expression,
    MemberExpression, SimpleAssignmentTarget, StringLiteral, TemplateElement, TemplateElementValue, TemplateLiteral,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Cap on the inlined string length. Inlining a multi-KB blob into many call
/// sites would explode the AST and slow the rest of the pipeline; for URL
/// pattern resolution, 256 chars is more than enough.
const MAX_INLINE_LEN: usize = 256;

/// Key for member-expression bindings. `this.foo` and `obj.foo` are tracked
/// separately because the receiver semantics differ: `this` is implicit and
/// resolves at call time, while `obj` is a concrete identifier.
#[derive(Clone, Eq, Hash, PartialEq, Debug)]
enum MemberKey {
    This(String),
    Bound(String, String),
}

#[derive(Default)]
pub struct DynConcatResolver {
    idents: FxHashMap<SymbolId, String>,
    members: FxHashMap<MemberKey, String>,
    poisoned_members: FxHashSet<MemberKey>,
    rewrites: usize,
    changed: bool,
}

impl DynConcatResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn rewrites(&self) -> usize {
        self.rewrites
    }

    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    pub fn reset_changed(&mut self) {
        self.changed = false;
    }

    fn lookup_ident(&self, symbol_id: SymbolId, ctx: &Ctx<'_>) -> Option<&str> {
        if ctx.scoping().symbol_is_mutated(symbol_id) {
            return None;
        }
        self.idents.get(&symbol_id).map(String::as_str)
    }

    fn lookup_member(&self, key: &MemberKey) -> Option<&str> {
        if self.poisoned_members.contains(key) {
            return None;
        }
        self.members.get(key).map(String::as_str)
    }
}

fn member_key_from_target(target: &AssignmentTarget<'_>) -> Option<MemberKey> {
    let SimpleAssignmentTarget::StaticMemberExpression(sme) = target.as_simple_assignment_target()? else {
        return None;
    };
    member_key_from_static_member(&sme.object, sme.property.name.as_str())
}

fn member_key_from_static_member(object: &Expression<'_>, prop: &str) -> Option<MemberKey> {
    match object {
        Expression::ThisExpression(_) => Some(MemberKey::This(prop.to_string())),
        Expression::Identifier(id) => Some(MemberKey::Bound(id.name.to_string(), prop.to_string())),
        _ => None,
    }
}

fn make_string_literal<'a>(value: &str, ctx: &Ctx<'a>) -> Expression<'a> {
    let interned = ctx.ast.str(value).clone_in(ctx.ast.allocator);
    Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        value: interned,
        raw: None,
        lone_surrogates: false,
    }))
}

fn try_resolve_to_string(expr: &Expression<'_>, resolver: &DynConcatResolver, ctx: &Ctx<'_>) -> Option<String> {
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

fn chain_has_string_evidence(expr: &Expression<'_>, resolver: &DynConcatResolver, ctx: &Ctx<'_>) -> bool {
    match expr {
        Expression::StringLiteral(_) => true,
        Expression::ParenthesizedExpression(p) => chain_has_string_evidence(&p.expression, resolver, ctx),
        Expression::TemplateLiteral(_) => true,
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

fn substitute_in_chain<'a>(expr: &mut Expression<'a>, resolver: &DynConcatResolver, ctx: &Ctx<'a>) -> usize {
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

fn fold_adjacent_literals<'a>(expr: &mut Expression<'a>, ctx: &Ctx<'a>) -> bool {
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

fn substitute_in_template<'a>(tmpl: &mut TemplateLiteral<'a>, resolver: &DynConcatResolver, ctx: &Ctx<'a>) -> usize {
    let mut count = 0;
    let mut new_quasis: Vec<TemplateElement<'a>> = Vec::with_capacity(tmpl.quasis.len());
    let mut new_exprs: Vec<Expression<'a>> = Vec::new();
    let mut pending_text = String::new();

    for (i, quasi) in tmpl.quasis.iter().enumerate() {
        pending_text.push_str(quasi.value.cooked.as_ref().unwrap_or(&quasi.value.raw).as_str());
        if i < tmpl.expressions.len() {
            let interp = &tmpl.expressions[i];
            let resolved = match interp {
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
            if let Some(value) = resolved {
                pending_text.push_str(&value);
                count += 1;
                continue;
            }
            let cooked = ctx.ast.str(&pending_text);
            let raw = ctx.ast.str(&pending_text);
            new_quasis.push(TemplateElement {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                tail: false,
                value: TemplateElementValue {
                    cooked: Some(cooked),
                    raw,
                },
                lone_surrogates: false,
            });
            new_exprs.push(interp.clone_in_with_semantic_ids(ctx.ast.allocator));
            pending_text.clear();
        }
    }

    let cooked = ctx.ast.str(&pending_text);
    let raw = ctx.ast.str(&pending_text);
    new_quasis.push(TemplateElement {
        node_id: Cell::new(NodeId::DUMMY),
        span: SPAN,
        tail: true,
        value: TemplateElementValue {
            cooked: Some(cooked),
            raw,
        },
        lone_surrogates: false,
    });

    if count > 0 {
        tmpl.quasis = ctx.ast.vec_from_iter(new_quasis);
        tmpl.expressions = ctx.ast.vec_from_iter(new_exprs);
    }
    count
}

impl<'a> Traverse<'a, DeobfuscateState> for DynConcatResolver {
    fn exit_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, ctx: &mut Ctx<'a>) {
        let Some(ident) = decl.id.get_binding_identifier() else {
            return;
        };
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        let Some(init) = &decl.init else {
            return;
        };
        if let Some(value) = try_resolve_to_string(init, self, ctx)
            && value.len() <= MAX_INLINE_LEN
            && self.idents.insert(symbol_id, value).is_none()
        {
            self.changed = true;
        }
    }

    fn exit_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, _ctx: &mut Ctx<'a>) {
        let key = match member_key_from_target(&assign.left) {
            Some(k) => k,
            None => return,
        };

        if !matches!(assign.operator, AssignmentOperator::Assign) {
            self.poisoned_members.insert(key);
            return;
        }

        if let Expression::StringLiteral(lit) = &assign.right
            && lit.value.len() <= MAX_INLINE_LEN
        {
            if let Some(prev) = self.members.insert(key.clone(), lit.value.as_str().to_string())
                && prev != lit.value.as_str()
            {
                self.poisoned_members.insert(key);
            } else {
                self.changed = true;
            }
            return;
        }

        self.poisoned_members.insert(key);
    }

    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        match expr {
            Expression::BinaryExpression(bin) => {
                if !matches!(bin.operator, BinaryOperator::Addition) {
                    return;
                }
                let resolver_view: &DynConcatResolver = self;
                if !chain_has_string_evidence(&bin.left, resolver_view, ctx)
                    && !chain_has_string_evidence(&bin.right, resolver_view, ctx)
                {
                    return;
                }
                let subs_l = substitute_in_chain(&mut bin.left, self, ctx);
                let subs_r = substitute_in_chain(&mut bin.right, self, ctx);
                let folded = fold_adjacent_literals(expr, ctx);
                let total = subs_l + subs_r;
                if total > 0 || folded {
                    if self.rewrites < 10 {
                        eprintln!("[AST/dyn-concat] inlined {total} ident(s) into '+' chain (folded={folded})");
                    }
                    self.rewrites = self.rewrites.saturating_add(total);
                    self.changed = true;
                }
            }
            Expression::TemplateLiteral(tmpl) => {
                let subs = substitute_in_template(tmpl, self, ctx);
                if subs > 0 {
                    if self.rewrites < 10 {
                        eprintln!("[AST/dyn-concat] inlined {subs} ident(s) into template literal");
                    }
                    self.rewrites = self.rewrites.saturating_add(subs);
                    self.changed = true;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        run_n(code, 5)
    }

    fn run_n(code: &str, max_iters: usize) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = DynConcatResolver::new();
        for _ in 0..max_iters {
            pass.reset_changed();
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
            if !pass.changed() {
                break;
            }
        }
        Codegen::new().build(&program).code
    }

    #[test]
    fn inlines_var_into_chain_with_literal_sibling() {
        let out = run("var p = \"/api/v1/\"; var u = p + e + \"/Submit\";");
        assert!(out.contains("\"/api/v1/\""), "got: {out}");
        assert!(
            !out.contains("p +") && !out.contains("p+"),
            "p should be inlined: {out}"
        );
    }

    #[test]
    fn inlines_var_into_chain_without_literal_sibling() {
        let out = run("var e = \"/Trading/Sleeve/\"; fetch(e + d);");
        assert!(out.contains("\"/Trading/Sleeve/\""), "got: {out}");
        assert!(!out.contains("e + d"), "e should be inlined: {out}");
    }

    #[test]
    fn inlines_let_string() {
        let out = run("let c = \"/QueryStudio/Queries/Summarize\"; var u = c + \"-suffix\";");
        assert!(out.contains("\"/QueryStudio/Queries/Summarize-suffix\""), "got: {out}");
    }

    #[test]
    fn inlines_const_string() {
        let out = run("const t = \"/Settings/WebOptions\"; var u = t + e;");
        assert!(out.contains("\"/Settings/WebOptions\""), "got: {out}");
    }

    #[test]
    fn skips_truly_numeric_addition() {
        let out = run("var n = a + b;");
        assert!(
            out.contains("a + b") || out.contains("a+b"),
            "no aliases means no change: {out}"
        );
    }

    #[test]
    fn refuses_to_inline_mutated_ident() {
        let out = run("var p = \"/a/\"; p = \"/b/\"; fetch(p + e);");
        assert!(!out.contains("\"/a/\" + e"), "mutated p must not be inlined: {out}");
    }

    #[test]
    fn handles_compound_assignment_as_mutation() {
        let out = run("var p = \"/a/\"; p += \"x\"; fetch(p + e);");
        assert!(!out.contains("\"/a/\" + e"), "compound assign must poison: {out}");
    }

    #[test]
    fn inlines_into_template_with_identifier() {
        let out = run("const BASE = \"/api/v1\"; var u = `${BASE}/Reporting/${id}`;");
        assert!(out.contains("/api/v1/Reporting/"), "got: {out}");
        assert!(!out.contains("${BASE}"), "BASE must be inlined: {out}");
        assert!(out.contains("${id}"), "unknown id remains: {out}");
    }

    #[test]
    fn inlines_this_property_into_chain() {
        let out = run("class Svc { ctor() { this.BASE_URL = \"/api/\"; } get(id) { return this.BASE_URL + id; } }");
        assert!(out.contains("\"/api/\""), "got: {out}");
    }

    #[test]
    fn inlines_this_property_into_template() {
        let out = run(
            "class Svc { ctor() { this.BASE_URL = \"/Security/Privileges\"; } get(a) { return `${this.BASE_URL}/${a}`; } }",
        );
        assert!(out.contains("/Security/Privileges/"), "got: {out}");
        assert!(
            !out.contains("${this.BASE_URL}"),
            "this.BASE_URL must be inlined: {out}"
        );
    }

    #[test]
    fn inlines_object_property_into_chain() {
        let out = run("o.baseUrl = \"/api/v1\"; fetch(o.baseUrl + \"/users\" + id);");
        assert!(
            out.contains("/api/v1/users"),
            "literal-fold should produce /api/v1/users: {out}"
        );
    }

    #[test]
    fn poisons_object_property_on_reassignment() {
        let out = run("o.baseUrl = \"/api/v1\"; o.baseUrl = \"/api/v2\"; fetch(o.baseUrl + \"/users\");");
        assert!(
            !out.contains("/api/v1/users") && !out.contains("/api/v2/users"),
            "got: {out}"
        );
    }

    #[test]
    fn ignores_computed_member_assignment() {
        let out = run("o[k] = \"/api/v1\"; fetch(o[k] + e);");
        assert!(out.contains("o[k]"), "computed member must be ignored: {out}");
    }

    #[test]
    fn resolves_chained_var_declarations() {
        let out = run("var BASE = \"/api\"; var V1 = BASE + \"/v1\"; fetch(V1 + e);");
        assert!(out.contains("\"/api/v1\""), "V1 should fold to /api/v1: {out}");
    }

    #[test]
    fn ignores_oversized_literal() {
        let huge = "x".repeat(MAX_INLINE_LEN + 10);
        let code = format!("var p = \"{huge}\"; var u = p + \"-\" + e;");
        let out = run(&code);
        assert!(
            out.contains("p +") || out.contains("p+"),
            "oversized must be skipped: {out}"
        );
    }

    #[test]
    fn folds_adjacent_string_literals_after_substitution() {
        let out = run("var a = \"/x/\"; var u = a + \"y\" + e;");
        assert!(out.contains("\"/x/y\""), "adjacent literals should fold: {out}");
    }

    #[test]
    fn handles_paren_wrapped_chain() {
        let out = run("var p = \"/api/\"; fetch((p) + (id));");
        assert!(out.contains("\"/api/\""), "paren-wrapped p should be inlined: {out}");
    }

    #[test]
    fn skips_destructured_binding() {
        let out = run("const { url } = config; fetch(url + e);");
        assert!(out.contains("url"), "destructured binding must not crash: {out}");
    }

    #[test]
    fn respects_inner_scope_shadow() {
        let out =
            run("var BASE = \"/outer/\"; function f() { var BASE = \"/inner/\"; return BASE + x; } var u = BASE + y;");
        assert!(out.contains("\"/outer/\""), "outer BASE inlined for u: {out}");
        assert!(out.contains("\"/inner/\""), "inner BASE inlined inside f: {out}");
    }

    #[test]
    fn deeply_nested_template_literal() {
        let out = run(
            "const BASE_URL_FOR_USER = \"/Authorization/User/Privileges\"; var u = `${BASE_URL_FOR_USER}/${a}?resetCache=${t}`;",
        );
        assert!(out.contains("/Authorization/User/Privileges/"), "got: {out}");
    }
}
