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

use oxc_ast::ast::{AssignmentExpression, AssignmentOperator, BinaryOperator, Expression, VariableDeclarator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub(super) const MAX_INLINE_LEN: usize = 256;

pub mod literal;
mod member;
mod resolve;
mod types;

use member::{MemberKey, member_key_from_target};
use resolve::{
    chain_has_string_evidence, fold_adjacent_literals, substitute_in_chain, substitute_in_template,
    try_resolve_to_string,
};

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
    pub const fn reset_changed(&mut self) {
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
        let Some(key) = member_key_from_target(&assign.left) else {
            return;
        };

        if !matches!(assign.operator, AssignmentOperator::Assign) {
            self.poisoned_members.insert(key);
            return;
        }

        if let Expression::StringLiteral(lit) = &assign.right
            && !lit.lone_surrogates
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
                // CWE-400 guard: the chain helpers below (chain_has_string_evidence /
                // substitute_in_chain / fold_adjacent_literals) each recurse the ENTIRE
                // `+` chain, so process only at the chain ROOT. If this `+` node is
                // itself an operand of a parent `+` chain, skip it — the root resolves
                // the whole chain in one O(n) pass. Without this, exit_expression's
                // per-node recursive scans are O(n²) on a long `a+a+a+...` chain,
                // a CPU/hang DoS on hostile input.
                let parent_is_plus_chain = match ctx.parent() {
                    Ancestor::BinaryExpressionLeft(p) => matches!(*p.operator(), BinaryOperator::Addition),
                    Ancestor::BinaryExpressionRight(p) => matches!(*p.operator(), BinaryOperator::Addition),
                    _ => false,
                };
                if parent_is_plus_chain {
                    return;
                }
                let resolver_view: &Self = self;
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
mod tests;
