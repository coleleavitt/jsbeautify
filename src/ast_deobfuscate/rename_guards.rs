//! Soundness guards shared by the by-`SymbolId` renaming passes
//! ([`super::bun::local_renamer`], [`super::bun::export_renamer`]).
//!
//! Renaming a binding's text is only sound when no *other* mechanism observes
//! its old name. Two such mechanisms survive a `SymbolId`-keyed rename and would
//! otherwise silently break runtime equivalence — neither is caught by
//! `node --check`, because both are *semantic*, not syntactic, faults:
//!
//! 1. **Direct `eval`** — `eval("…H…")` resolves `H` by *name* against the live
//!    scope chain, not by `SymbolId`, so renaming `H` leaves a dangling
//!    reference inside the eval string. Every binding a direct eval can observe
//!    (its own scope and all ancestors) is therefore unsafe to rename. Only the
//!    bare `eval(...)` callee form (parenthesized or not) is a direct eval; indirect forms
//!    (`(0,eval)(x)`, `window.eval(x)`, `new Function(...)`) run in global scope
//!    and cannot see locals, so they are intentionally *not* flagged.
//! 2. **Native ES `export <decl>` / `export { x }`** — `export function H(){}`,
//!    `export const K = 2`, and `export { H }` expose the *binding name itself*
//!    as the module's public surface (there is no separate alias to preserve
//!    it). Renaming the binding changes what importers see. `export default …`
//!    is safe (the public name is always `default`), and `export … from "mod"`
//!    re-exports a foreign binding (nothing local is exposed by name).
//!
//! 3. **Classic-script globals** — in a script (not a module or `CommonJS`
//!    body), every top-level binding is shared: `var` and `function` become
//!    properties of the global object, `let`/`const`/`class` live in the global
//!    lexical scope, and other scripts, `window.x`, `this.x` and
//!    `globalThis["x"]` all read them by name. See [`is_global_binding`].
//! 4. **Annex B block functions** — a sloppy-mode `function f(){}` inside a
//!    block also assigns the enclosing scope's `var f`. See
//!    [`is_annex_b_function`].
//!
//! [`RenameGuardCollector`] runs one traversal and, via
//! [`RenameGuardCollector::into_guarded`], returns the set of [`SymbolId`]s that
//! every rename plan must exclude. Excluded symbols keep their original name, so
//! they still *reserve* that name against collisions in the rename planners.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingIdentifier, CallExpression, Declaration, ExportDeclaration, ExportNamedDeclaration, Expression, Function,
    IdentifierReference, ModuleExportName, Program,
};
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Whether `symbol` is a top-level binding of a classic script, observable by
/// name from outside the file, so no pass may rename or remove it.
///
/// Sloppy-mode block functions that Annex B hoists to the global object are
/// already bound in the root scope by `oxc_semantic`.
#[must_use]
pub fn is_global_binding(source_type: SourceType, scoping: &Scoping, symbol: SymbolId) -> bool {
    source_type.is_script() && scoping.symbol_scope_id(symbol) == scoping.root_scope_id()
}

fn program_source_type<State>(ctx: &TraverseCtx<'_, State>) -> Option<SourceType> {
    match ctx.ancestors().last()? {
        Ancestor::ProgramBody(program) => Some(*program.source_type()),
        Ancestor::ProgramDirectives(program) => Some(*program.source_type()),
        Ancestor::ProgramHashbang(program) => Some(*program.source_type()),
        _ => None,
    }
}

/// [`is_global_binding`] for a pass in the middle of a traversal, which reads
/// the source type from the enclosing `Program`.
#[must_use]
pub fn is_global_symbol<State>(ctx: &TraverseCtx<'_, State>, symbol: SymbolId) -> bool {
    program_source_type(ctx).is_some_and(|source_type| is_global_binding(source_type, ctx.scoping(), symbol))
}

/// Whether `func` is a sloppy-mode function declaration nested in a block or
/// an `if`/`switch`/label body. Annex B also assigns it to a same-named `var`
/// of the enclosing function or global scope, so its name is observable
/// outside the block even where `oxc_semantic` binds it block-scoped.
///
/// Call it from `enter_function` or from `*_statement` on the declaration, so
/// the current scope is the one enclosing the function.
#[must_use]
pub fn is_annex_b_function<State>(ctx: &TraverseCtx<'_, State>, func: &Function<'_>) -> bool {
    func.is_declaration()
        && !ctx.current_scope_flags().is_strict_mode()
        && !matches!(
            ctx.parent(),
            Ancestor::ProgramBody(_) | Ancestor::FunctionBodyStatements(_)
        )
}

/// Whether the declaration of `id` binds a classic-script global.
#[must_use]
pub fn binds_global<State>(ctx: &TraverseCtx<'_, State>, id: &BindingIdentifier<'_>) -> bool {
    id.symbol_id.get().is_some_and(|symbol| is_global_symbol(ctx, symbol))
}

/// Collects the soundness-guard inputs in a single read-only traversal.
#[derive(Default)]
pub struct RenameGuardCollector {
    /// Scopes that lexically contain a direct `eval(...)` call.
    eval_scopes: Vec<ScopeId>,
    /// Bindings whose name is observable as a public ES module export, or
    /// through Annex B block-function hoisting.
    preserved: FxHashSet<SymbolId>,
    source_type: SourceType,
}

impl RenameGuardCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the collected signals into the set of [`SymbolId`]s that must
    /// never be renamed: every native-export binding, every classic-script
    /// global, every Annex B block function, plus every symbol visible to a direct `eval` (the eval's own
    /// scope and all of its ancestors).
    ///
    /// `scoping` must describe the same (unmodified) program the collector
    /// traversed; `SymbolId`/`ScopeId` assignment is deterministic across
    /// rebuilds, which the surrounding pipeline already relies on.
    #[must_use]
    pub fn into_guarded(self, scoping: &Scoping) -> FxHashSet<SymbolId> {
        let mut guarded = self.preserved;
        if self.source_type.is_script() {
            guarded.extend(scoping.get_bindings(scoping.root_scope_id()).values().copied());
        }

        if !self.eval_scopes.is_empty() {
            let mut poisoned: FxHashSet<ScopeId> = FxHashSet::default();
            for &scope in &self.eval_scopes {
                poisoned.extend(scoping.scope_ancestors(scope));
            }
            for sym in scoping.symbol_ids() {
                if poisoned.contains(&scoping.symbol_scope_id(sym)) {
                    guarded.insert(sym);
                }
            }
        }

        guarded
    }
}

/// Runs [`RenameGuardCollector`] over `program` and resolves its guarded set.
pub fn guarded_symbols<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> FxHashSet<SymbolId> {
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    collector.into_guarded(&ctx.into_scoping())
}

impl<'a> Traverse<'a, DeobfuscateState> for RenameGuardCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.source_type = program.source_type;
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if is_annex_b_function(ctx, func)
            && let Some(symbol_id) = func.id.as_ref().and_then(|id| id.symbol_id.get())
        {
            self.preserved.insert(symbol_id);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        // Direct eval only: a bare `eval(...)` or `(eval)(...)` callee identifier.
        if let Expression::Identifier(id) = call.callee.without_parentheses()
            && id.name == "eval"
        {
            self.eval_scopes.push(ctx.current_scope_id());
        }
    }

    fn enter_export_declaration(&mut self, export: &mut ExportDeclaration<'a>, _ctx: &mut Ctx<'a>) {
        let decl = &export.declaration;
        match decl {
            Declaration::VariableDeclaration(var) => {
                for declarator in &var.declarations {
                    for binding in declarator.id.get_binding_identifiers() {
                        if let Some(symbol_id) = binding.symbol_id.get() {
                            self.preserved.insert(symbol_id);
                        }
                    }
                }
            }
            _ => {
                if let Some(id) = decl.id()
                    && let Some(symbol_id) = id.symbol_id.get()
                {
                    self.preserved.insert(symbol_id);
                }
            }
        }
    }

    fn enter_export_named_declaration(&mut self, export: &mut ExportNamedDeclaration<'a>, ctx: &mut Ctx<'a>) {
        for spec in &export.specifiers {
            if let ModuleExportName::IdentifierReference(local) = &spec.local
                && let Some(symbol_id) = ctx.scoping().get_reference(local.reference_id()).symbol_id()
            {
                self.preserved.insert(symbol_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    /// Run the collector on `code` and return the guarded names (resolved back
    /// to their symbol's text for easy assertions).
    fn guarded_names(code: &str) -> FxHashSet<String> {
        guarded_names_as(code, SourceType::mjs())
    }

    fn guarded_names_as(code: &str, source_type: SourceType) -> FxHashSet<String> {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, source_type).parse();
        let mut prog = parsed.program;

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        let mut collector = RenameGuardCollector::new();
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        drop(ctx);

        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        collector
            .into_guarded(&scoping)
            .into_iter()
            .map(|sym| scoping.symbol_name(sym).to_string())
            .collect()
    }

    #[test]
    fn flags_direct_eval_scope_chain() {
        // Direct eval inside `f` can observe `f`'s param `H` and the outer `g`.
        let names = guarded_names("function g(){ function f(H){ return eval('H') } }");
        assert!(names.contains("H"), "eval-visible param must be guarded: {names:?}");
    }

    #[test]
    fn flags_parenthesized_direct_eval() {
        let names = guarded_names("function f(){ (eval)('x'); var x }");
        assert!(names.contains("x"), "`(eval)(...)` is a direct eval: {names:?}");
    }

    #[test]
    fn ignores_indirect_eval() {
        // `(0,eval)(...)` is indirect eval (runs in global scope), so locals are
        // not observable and remain renamable.
        let names = guarded_names("function f(H){ return (0,eval)('H') }");
        assert!(!names.contains("H"), "indirect eval must not guard locals: {names:?}");
    }

    #[test]
    fn flags_native_export_declarations() {
        let names = guarded_names("export function H(){} export const K = 2; let q; export { q };");
        assert!(names.contains("H"), "exported function name must be guarded: {names:?}");
        assert!(names.contains("K"), "exported const name must be guarded: {names:?}");
        assert!(names.contains("q"), "bare-specifier export must be guarded: {names:?}");
    }

    #[test]
    fn does_not_flag_default_export_or_plain_locals() {
        // `export default` exposes only `default`, so `H` stays renamable; the
        // plain inner local `K` is never exported.
        let names = guarded_names("export default function H(){ let K = 1; return K }");
        assert!(
            !names.contains("H"),
            "default export local should not be guarded: {names:?}"
        );
        assert!(!names.contains("K"), "plain local should not be guarded: {names:?}");
    }

    #[test]
    fn flags_every_script_global() {
        let code = "var v; let l = 1; const c = 2; class K {} function f(p) { var inner; } if (v) { var nested; }";
        let names = guarded_names_as(code, SourceType::script());
        for global in ["v", "l", "c", "K", "f", "nested"] {
            assert!(
                names.contains(global),
                "script global {global} must be guarded: {names:?}"
            );
        }
        assert!(!names.contains("p") && !names.contains("inner"), "{names:?}");
    }

    #[test]
    fn module_and_commonjs_top_level_is_not_flagged() {
        let code = "var v; function f() {}";
        assert!(guarded_names_as(code, SourceType::mjs()).is_empty());
        assert!(guarded_names_as(code, SourceType::cjs()).is_empty());
    }

    #[test]
    fn flags_sloppy_block_functions() {
        let code = "function g() { { function f() {} } { function f() {} } if (g) function h() {} }";
        let names = guarded_names_as(code, SourceType::cjs());
        assert!(names.contains("f") && names.contains("h"), "{names:?}");
        assert!(!names.contains("g"), "{names:?}");
        let strict = guarded_names_as(code, SourceType::mjs());
        assert!(
            !strict.contains("f"),
            "strict block functions are block-scoped: {strict:?}"
        );
    }
}
