//! Heuristic readable-name inference for minified locals.
//!
//! The slot renamer ([`super::bun_local_renamer`]) already rewrites minified
//! locals scope-safely — but to opaque `v0`/`v1` slots. This pass *proposes* a
//! readable name per binding, which the slot renamer then applies (falling back
//! to a slot when a proposal collides or is blocked). Names are inferred
//! **deterministically** from the AST so output is diff-stable.
//!
//! Inference runs at the same pipeline seam as the apply (Phase 12.5) so both
//! share one [`SymbolId`] space — collecting earlier would key proposals to a
//! symbol set that the intervening structural passes invalidate.
//!
//! ## Heuristics (highest precedence first)
//!
//! | Prio | Signal | Name |
//! |------|--------|------|
//! | 10 | `catch (X)` | `error` |
//! | 9  | `for (X = 0; …)` loop counter | `i`, `j`, `k`, … by nesting depth |
//! | 9  | `for (X of …)` / `for (X in …)` | `item` / `key` |
//! | 8  | `X = require("m")` | camelCased module basename |
//! | 7  | `X = new Foo()` | lowerCamel(`Foo`) instance name |
//! | 5  | `X = []` / `{}` / `JSON.parse(…)` / regex | `arr` / `obj` / `data` / `re` |
//! | 5  | `X = typeof y === "…"` / `!!y` / `Boolean(y)` | `isString` / `flag` |
//! | 4  | `X = "…"` / number | `str` / `num` |
//!
//! ## Capture safety
//!
//! It also records, per scope, every UNRESOLVED reference name (free vars /
//! globals). The slot renamer blocks proposals against these so renaming a local
//! to e.g. `crypto` can never capture a free `crypto` reference — a silent
//! behaviour change a `v{n}` slot could never cause.

use oxc_ast::ast::{
    Argument, BindingPattern, CatchClause, Expression, ForInStatement, ForOfStatement, ForStatement, ForStatementInit,
    IdentifierReference, UnaryOperator, VariableDeclarator,
};
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Loop-counter names by nesting depth. Capped at three: `i`/`j`/`k` are in the
/// slot renamer's meaningful-short allowlist (so re-running `--deobfuscate` is
/// idempotent — they're never re-flagged as minified), whereas `l`/`m`/`n`
/// are not, so deeper loops fall back to slots instead.
const LOOP_NAMES: &[&str] = &["i", "j", "k"];

/// Inferred readable-name proposals + per-scope free-reference names (for
/// capture-safe collision blocking).
#[derive(Default)]
pub struct HeuristicNameCollector {
    /// `SymbolId` → (priority, proposed base name). Higher priority wins.
    proposals: FxHashMap<SymbolId, (u8, String)>,
    /// Scope → unresolved (free/global) reference names occurring directly in it.
    free_names: FxHashMap<ScopeId, FxHashSet<String>>,
    for_depth: usize,
}

impl HeuristicNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume into the proposal map (base names) and the per-scope free-name map.
    #[must_use]
    pub fn into_parts(self) -> (FxHashMap<SymbolId, String>, FxHashMap<ScopeId, FxHashSet<String>>) {
        let proposals = self
            .proposals
            .into_iter()
            .map(|(sym, (_p, name))| (sym, name))
            .collect();
        (proposals, self.free_names)
    }

    fn propose(&mut self, sym: SymbolId, priority: u8, name: String) {
        if name.is_empty() {
            return;
        }
        match self.proposals.get(&sym) {
            Some((p, _)) if *p >= priority => {}
            _ => {
                self.proposals.insert(sym, (priority, name));
            }
        }
    }
}

/// The `SymbolId` of a plain `BindingIdentifier` pattern (destructuring is skipped).
fn binding_symbol(pat: &BindingPattern<'_>) -> Option<SymbolId> {
    match pat {
        BindingPattern::BindingIdentifier(b) => b.symbol_id.get(),
        _ => None,
    }
}

/// `"./utils/foo-bar.js"` → `fooBar`, `"@scope/pkg"` → `pkg`, `"crypto"` → `crypto`.
fn module_to_name(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let base = base.split('.').next().unwrap_or(base); // strip extension
    // camelCase across non-alphanumeric separators.
    let mut out = String::new();
    let mut upper_next = false;
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper_next {
                out.extend(ch.to_uppercase());
                upper_next = false;
            } else {
                out.push(ch);
            }
        } else {
            upper_next = !out.is_empty();
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// PascalCase/acronym → lowerCamel: `URLSearchParams` → `urlSearchParams`,
/// `Map` → `map`, `URL` → `url`, `HTMLElement` → `htmlElement`.
fn lower_camel(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let leading_upper = chars.iter().take_while(|c| c.is_uppercase()).count();
    if leading_upper <= 1 {
        let mut out = String::new();
        out.extend(chars[0].to_lowercase());
        out.extend(&chars[1..]);
        out
    } else if leading_upper == chars.len() {
        s.to_lowercase()
    } else {
        // Acronym run then a new word: lowercase all but the last upper.
        let mut out: String = chars[..leading_upper - 1]
            .iter()
            .flat_map(|c| c.to_lowercase())
            .collect();
        out.extend(&chars[leading_upper - 1..]);
        out
    }
}

/// `JSON.parse(…)`?
fn is_json_parse(expr: &Expression<'_>) -> bool {
    if let Expression::CallExpression(call) = expr
        && let Some(member) = call.callee.as_member_expression()
        && let Expression::Identifier(obj) = member.object()
        && obj.name == "JSON"
        && member.static_property_name() == Some("parse")
    {
        return true;
    }
    false
}

/// Infer a `(priority, name)` from a declarator initializer.
fn infer_from_init(init: &Expression<'_>) -> Option<(u8, String)> {
    match init {
        // X = require("module")
        Expression::CallExpression(call) => {
            if let Expression::Identifier(callee) = &call.callee
                && callee.name == "require"
                && let Some(Argument::StringLiteral(s)) = call.arguments.first()
            {
                let n = module_to_name(s.value.as_str());
                if !n.is_empty() {
                    return Some((8, n));
                }
            }
            if is_json_parse(init) {
                return Some((5, "data".to_string()));
            }
            // Boolean(x)
            if let Expression::Identifier(callee) = &call.callee
                && callee.name == "Boolean"
            {
                return Some((5, "flag".to_string()));
            }
            None
        }
        // X = new Foo(...)
        Expression::NewExpression(new_expr) => {
            let ctor = match &new_expr.callee {
                Expression::Identifier(id) => Some(id.name.as_str()),
                e => e
                    .as_member_expression()
                    .and_then(oxc_ast::ast::MemberExpression::static_property_name),
            }?;
            let n = lower_camel(ctor);
            (!n.is_empty()).then_some((7, n))
        }
        Expression::ArrayExpression(_) => Some((5, "arr".to_string())),
        Expression::ObjectExpression(_) => Some((5, "obj".to_string())),
        Expression::RegExpLiteral(_) => Some((5, "regex".to_string())),
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => Some((4, "str".to_string())),
        Expression::NumericLiteral(_) => Some((4, "num".to_string())),
        // X = typeof y === "string"  →  isString
        Expression::BinaryExpression(bin) => {
            let is_typeof =
                |e: &Expression<'_>| matches!(e, Expression::UnaryExpression(u) if u.operator == UnaryOperator::Typeof);
            if !is_typeof(&bin.left) && !is_typeof(&bin.right) {
                return None;
            }
            let ty = if let Expression::StringLiteral(s) = &bin.right {
                Some(s.value.as_str())
            } else if let Expression::StringLiteral(s) = &bin.left {
                Some(s.value.as_str())
            } else {
                None
            };
            let name = match ty {
                Some(t) if !t.is_empty() => {
                    format!("is{}{}", t[..1].to_uppercase(), &t[1..])
                }
                _ => "isType".to_string(),
            };
            Some((5, name))
        }
        // X = !!y
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
            if matches!(&u.argument, Expression::UnaryExpression(inner) if inner.operator == UnaryOperator::LogicalNot)
            {
                return Some((5, "flag".to_string()));
            }
            None
        }
        _ => None,
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for HeuristicNameCollector {
    fn enter_catch_clause(&mut self, clause: &mut CatchClause<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(param) = &clause.param
            && let Some(sym) = binding_symbol(&param.pattern)
        {
            self.propose(sym, 10, "error".to_string());
        }
    }

    fn enter_for_statement(&mut self, stmt: &mut ForStatement<'a>, _ctx: &mut Ctx<'a>) {
        // Only name the first three nesting levels (i/j/k); deeper loops keep
        // slots (see LOOP_NAMES).
        if let Some(name) = LOOP_NAMES.get(self.for_depth).copied()
            // for (let X = …; …) — the dominant minified counter form.
            && let Some(ForStatementInit::VariableDeclaration(decl)) = &stmt.init
            && let Some(first) = decl.declarations.first()
            && let Some(sym) = binding_symbol(&first.id)
        {
            self.propose(sym, 9, name.to_string());
        }
        self.for_depth += 1;
    }

    fn exit_for_statement(&mut self, _stmt: &mut ForStatement<'a>, _ctx: &mut Ctx<'a>) {
        self.for_depth = self.for_depth.saturating_sub(1);
    }

    fn enter_for_of_statement(&mut self, stmt: &mut ForOfStatement<'a>, _ctx: &mut Ctx<'a>) {
        if let oxc_ast::ast::ForStatementLeft::VariableDeclaration(decl) = &stmt.left
            && let Some(first) = decl.declarations.first()
            && let Some(sym) = binding_symbol(&first.id)
        {
            self.propose(sym, 8, "item".to_string());
        }
    }

    fn enter_for_in_statement(&mut self, stmt: &mut ForInStatement<'a>, _ctx: &mut Ctx<'a>) {
        if let oxc_ast::ast::ForStatementLeft::VariableDeclaration(decl) = &stmt.left
            && let Some(first) = decl.declarations.first()
            && let Some(sym) = binding_symbol(&first.id)
        {
            self.propose(sym, 8, "key".to_string());
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let Some(sym) = binding_symbol(&decl.id) else {
            return;
        };
        let Some(init) = &decl.init else { return };
        if let Some((prio, name)) = infer_from_init(init) {
            self.propose(sym, prio, name);
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        // Unresolved reference = a free var / global. Record it so the slot
        // renamer never picks a readable name that would capture it.
        if ctx.scoping().get_reference(ident.reference_id()).symbol_id().is_none() {
            self.free_names
                .entry(ctx.current_scope_id())
                .or_default()
                .insert(ident.name.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_camelcases() {
        assert_eq!(module_to_name("crypto"), "crypto");
        assert_eq!(module_to_name("./utils/foo-bar.js"), "fooBar");
        assert_eq!(module_to_name("@scope/pkg"), "pkg");
        assert_eq!(module_to_name("react-dom"), "reactDom");
        assert_eq!(module_to_name("3d-engine"), "_3dEngine");
    }

    #[test]
    fn lower_camel_handles_acronyms() {
        assert_eq!(lower_camel("Map"), "map");
        assert_eq!(lower_camel("URL"), "url");
        assert_eq!(lower_camel("URLSearchParams"), "urlSearchParams");
        assert_eq!(lower_camel("HTMLElement"), "htmlElement");
        assert_eq!(lower_camel("EventEmitter"), "eventEmitter");
    }
}
