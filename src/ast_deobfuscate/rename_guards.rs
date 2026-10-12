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
//!    (its own scope and all ancestors) is therefore unsafe to rename, except
//!    that `eval("name")` reads one identifier: [`RenameGuardCollector::into_guards`]
//!    guards only the binding `name` resolves to and reserves `name`. Only the
//!    bare `eval(...)` callee form (parenthesized or not) is a direct eval; indirect forms
//!    (`(0,eval)(x)`, `window.eval(x)`, `new Function(...)`) run in global scope
//!    and cannot see locals, so they are intentionally *not* flagged.
//! 2. **Native ES `export <decl>` / `export { x }`** — `export function H(){}`
//!    and `export const K = 2` expose the *binding name itself* as the module's
//!    public surface (there is no separate alias to preserve it). Renaming the
//!    binding changes what importers see. `export default …` and
//!    `export { H as default }` are safe (the public name is always
//!    `default`), and `export … from "mod"` re-exports a foreign binding
//!    (nothing local is exposed by name). A specifier `export { H }` keeps its
//!    exported name when `H` is renamed (codegen prints `export { G as H }`),
//!    but other passes would make that public name drift away from the
//!    local, so such bindings stay guarded and are reported in
//!    [`RenameGuards::specifier_exports`] for the passes that name them.
//!
//! 3. **Classic-script globals** — in a script (not a module or `CommonJS`
//!    body), every top-level binding is shared: `var` and `function` become
//!    properties of the global object, `let`/`const`/`class` live in the global
//!    lexical scope, and other scripts, `window.x`, `this.x` and
//!    `globalThis["x"]` all read them by name. See [`is_global_binding`].
//! 4. **Annex B block functions** — a sloppy-mode `function f(){}` inside a
//!    block also assigns the enclosing scope's `var f`. See
//!    [`is_annex_b_function`]. Where `oxc_semantic` binds both as one symbol,
//!    a planner that keeps the new name free up to the var scope may rename
//!    it ([`RenameGuardCollector::renaming_annex_b_functions`]).
//!
//! [`RenameGuardCollector`] runs one traversal and, via
//! [`RenameGuardCollector::into_guarded`], returns the set of [`SymbolId`]s that
//! every rename plan must exclude. Excluded symbols keep their original name, so
//! they still *reserve* that name against collisions in the rename planners.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingIdentifier, CallExpression, Declaration, ExportDeclaration,
    ExportNamedDeclaration, Expression, Function, IdentifierReference, LogicalOperator, ModuleExportName,
    ObjectExpression, ObjectPropertyKind, Program, Statement,
};
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::SourceType;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::bun::export_renamer::names::{is_reserved_word, is_valid_identifier};
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

/// Whether `func` is a sloppy-mode plain function declaration nested in a
/// block or an `if`/`switch`/label body (Annex B.3.3 leaves async functions
/// and generators block-scoped). Annex B also assigns it to a same-named `var`
/// of the enclosing function or global scope, so its name is observable
/// outside the block even where `oxc_semantic` binds it block-scoped.
///
/// Call it from `enter_function` or from `*_statement` on the declaration, so
/// the current scope is the one enclosing the function.
#[must_use]
pub fn is_annex_b_function<State>(ctx: &TraverseCtx<'_, State>, func: &Function<'_>) -> bool {
    func.is_declaration()
        && !func.r#async
        && !func.generator
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

/// Names the program reads as free globals, leaving out the references in the
/// export-name hint esbuild appends to `CommonJS` output,
/// `0 && (module.exports = { a, b })`. The hint is never evaluated (it exists
/// for Node's cjs-module-lexer), so renaming a binding to one of its names
/// cannot change what the program does.
#[must_use]
pub fn free_global_names<'s>(program: &Program<'_>, scoping: &'s Scoping) -> FxHashSet<&'s str> {
    let hint: FxHashSet<ReferenceId> = program
        .body
        .iter()
        .filter_map(cjs_export_names_hint)
        .flat_map(|object| &object.properties)
        .filter_map(|property| match property {
            ObjectPropertyKind::ObjectProperty(property) => match &property.value {
                Expression::Identifier(id) => id.reference_id.get(),
                _ => None,
            },
            ObjectPropertyKind::SpreadProperty(_) => None,
        })
        .collect();
    scoping
        .root_unresolved_references()
        .iter()
        .filter(|(_, references)| references.iter().any(|reference| !hint.contains(reference)))
        .map(|(name, _)| name.as_str())
        .collect()
}

/// The object of a `0 && (module.exports = {...})` statement.
fn cjs_export_names_hint<'p, 'a>(statement: &'p Statement<'a>) -> Option<&'p ObjectExpression<'a>> {
    let Statement::ExpressionStatement(statement) = statement else {
        return None;
    };
    let Expression::LogicalExpression(logical) = statement.expression.without_parentheses() else {
        return None;
    };
    if logical.operator != LogicalOperator::And
        || !matches!(&logical.left, Expression::NumericLiteral(zero) if zero.value == 0.0)
    {
        return None;
    }
    let Expression::AssignmentExpression(assign) = logical.right.without_parentheses() else {
        return None;
    };
    let AssignmentTarget::StaticMemberExpression(target) = &assign.left else {
        return None;
    };
    let Expression::Identifier(module) = &target.object else {
        return None;
    };
    if module.name != "module" || target.property.name != "exports" {
        return None;
    }
    let Expression::ObjectExpression(object) = &assign.right else {
        return None;
    };
    Some(object)
}

/// The identifier `name` when `call` is `eval("name")`: the evaluated code is
/// one identifier reference, so the eval reads that name and nothing else.
fn eval_identifier<'c>(call: &'c CallExpression<'_>) -> Option<&'c str> {
    let [Argument::StringLiteral(code)] = call.arguments.as_slice() else {
        return None;
    };
    let name = code.value.as_str();
    (is_valid_identifier(name) && !is_reserved_word(name)).then_some(name)
}

/// Collects the soundness-guard inputs in a single read-only traversal.
#[derive(Default)]
pub struct RenameGuardCollector {
    /// Scopes that lexically contain a direct `eval(...)` call.
    eval_scopes: Vec<ScopeId>,
    /// Direct evals whose code is a single identifier, such as `eval("require")`,
    /// with the scope they run in. Such an eval reads only that one name.
    identifier_evals: Vec<(ScopeId, String)>,
    /// Bindings whose name is observable as a public ES module export, or
    /// through Annex B block-function hoisting.
    preserved: FxHashSet<SymbolId>,
    /// Bindings exported by an `export { local as name }` specifier.
    specifier_exports: FxHashSet<SymbolId>,
    /// Annex B block functions `oxc_semantic` bound once, in the var scope, with
    /// the block that declares them. Guarded unless [`Self::renaming_annex_b_functions`].
    annex_b_hoisted: FxHashMap<SymbolId, ScopeId>,
    rename_annex_b: bool,
    source_type: SourceType,
}

impl RenameGuardCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Leave Annex B block functions renamable when `oxc_semantic` bound the
    /// block function and its hoisted `var` as one symbol: renaming that symbol
    /// renames both. The caller must keep the new name free in the scopes from
    /// the declaring block up to the var scope, as [`Self::annex_b_blocks`]
    /// lists them, or Annex B would not create the `var`.
    #[must_use]
    pub const fn renaming_annex_b_functions(mut self) -> Self {
        self.rename_annex_b = true;
        self
    }

    /// The renamable Annex B block functions and the blocks declaring them.
    #[must_use]
    pub fn annex_b_blocks(&self) -> FxHashMap<SymbolId, ScopeId> {
        self.annex_b_hoisted
            .iter()
            .filter(|(symbol, _)| !self.preserved.contains(symbol))
            .map(|(&symbol, &block)| (symbol, block))
            .collect()
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
    pub fn into_guarded(mut self, scoping: &Scoping) -> FxHashSet<SymbolId> {
        let identifier_evals = std::mem::take(&mut self.identifier_evals);
        self.eval_scopes
            .extend(identifier_evals.into_iter().map(|(scope, _)| scope));
        self.into_guards(scoping).symbols
    }

    /// Like [`Self::into_guarded`], except that a direct eval of a single
    /// identifier (`eval("require")`) guards only the binding that name
    /// resolves to and reports the name in [`RenameGuards::eval_names`]. The
    /// caller must then never give any binding one of those names, or the
    /// eval would read the renamed binding instead.
    #[must_use]
    pub fn into_guards(self, scoping: &Scoping) -> RenameGuards {
        let mut symbols = self.preserved;
        if !self.rename_annex_b {
            symbols.extend(self.annex_b_hoisted.keys());
        }
        if self.source_type.is_script() {
            symbols.extend(scoping.get_bindings(scoping.root_scope_id()).values().copied());
        }

        if !self.eval_scopes.is_empty() {
            let mut poisoned: FxHashSet<ScopeId> = FxHashSet::default();
            for &scope in &self.eval_scopes {
                poisoned.extend(scoping.scope_ancestors(scope));
            }
            for sym in scoping.symbol_ids() {
                if poisoned.contains(&scoping.symbol_scope_id(sym)) {
                    symbols.insert(sym);
                }
            }
        }

        let mut eval_names = FxHashSet::default();
        for (scope, name) in self.identifier_evals {
            symbols.extend(scoping.find_binding(scope, name.as_str().into()));
            eval_names.insert(name);
        }

        let specifier_exports: FxHashSet<SymbolId> = self
            .specifier_exports
            .into_iter()
            .filter(|symbol| !symbols.contains(symbol))
            .collect();
        symbols.extend(&specifier_exports);

        RenameGuards {
            symbols,
            eval_names,
            specifier_exports,
        }
    }
}

/// The result of [`RenameGuardCollector::into_guards`].
pub struct RenameGuards {
    /// Bindings that must keep their name.
    pub symbols: FxHashSet<SymbolId>,
    /// Names read by `eval("name")`, which no binding may be renamed to.
    pub eval_names: FxHashSet<String>,
    /// The bindings in [`Self::symbols`] guarded only because an
    /// `export { local as name }` specifier exports them. A naming pass may
    /// rename them anyway: the specifier keeps every public name.
    pub specifier_exports: FxHashSet<SymbolId>,
}

/// Runs [`RenameGuardCollector`] over `program` and resolves its
/// [`RenameGuards`].
pub fn guards<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> RenameGuards {
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    collector.into_guards(&ctx.into_scoping())
}

impl<'a> Traverse<'a, DeobfuscateState> for RenameGuardCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.source_type = program.source_type;
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if !is_annex_b_function(ctx, func) {
            return;
        }
        let Some(id) = &func.id else { return };
        let Some(symbol_id) = id.symbol_id.get() else { return };
        let block = ctx.current_scope_id();
        if ctx.scoping().symbol_scope_id(symbol_id) == block {
            // `oxc_semantic` left the block binding apart from the hoisted `var`
            // because the var scope already binds the name: two symbols name one
            // variable at runtime.
            self.preserved.insert(symbol_id);
            if let Some(parent) = ctx.scoping().scope_parent_id(block)
                && let Some(outer) = ctx.scoping().find_binding(parent, id.name.as_str().into())
            {
                self.preserved.insert(outer);
            }
        } else {
            self.annex_b_hoisted.insert(symbol_id, block);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        // Direct eval only: a bare `eval(...)` or `(eval)(...)` callee identifier.
        if let Expression::Identifier(id) = call.callee.without_parentheses()
            && id.name == "eval"
        {
            let scope = ctx.current_scope_id();
            match eval_identifier(call) {
                Some(name) => self.identifier_evals.push((scope, name.to_string())),
                None => self.eval_scopes.push(scope),
            }
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
                && spec.exported.name() != "default"
                && let Some(symbol_id) = ctx.scoping().get_reference(local.reference_id()).symbol_id()
            {
                self.specifier_exports.insert(symbol_id);
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

    fn guards_of(code: &str) -> (FxHashSet<String>, FxHashSet<String>) {
        let alloc = Allocator::default();
        let mut prog = Parser::new(&alloc, code, SourceType::cjs()).parse().program;
        let guards = super::guards(&alloc, &mut prog);
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let names = guards
            .symbols
            .into_iter()
            .map(|sym| scoping.symbol_name(sym).to_string())
            .collect();
        (names, guards.eval_names)
    }

    #[test]
    fn identifier_eval_guards_only_the_binding_it_reads() {
        let code = "var a, b; function f(c) { return eval('a'); }";
        let (names, eval_names) = guards_of(code);
        assert_eq!(names, FxHashSet::from_iter(["a".to_string()]));
        assert_eq!(eval_names, FxHashSet::from_iter(["a".to_string()]));
        let conservative = guarded_names_as(code, SourceType::cjs());
        assert!(
            ["a", "b", "c", "f"].iter().all(|name| conservative.contains(*name)),
            "{conservative:?}"
        );
    }

    #[test]
    fn identifier_eval_of_a_global_reserves_its_name() {
        let (names, eval_names) = guards_of("var a; function f() { return eval('require')('fs'); }");
        assert!(names.is_empty(), "{names:?}");
        assert_eq!(eval_names, FxHashSet::from_iter(["require".to_string()]));
    }

    #[test]
    fn other_literal_evals_still_guard_the_scope_chain() {
        for code in [
            "var a; function f() { return eval('a + 1'); }",
            "var a; function f() { return eval('var a'); }",
            "var a; function f() { return eval('this'); }",
            "var a; function f() { return eval('a', 1); }",
        ] {
            let (names, eval_names) = guards_of(code);
            assert!(names.contains("a") && names.contains("f"), "{code}: {names:?}");
            assert!(eval_names.is_empty(), "{code}: {eval_names:?}");
        }
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
    fn reports_bindings_only_a_specifier_exports() {
        let specifier_exports = |code: &str| -> FxHashSet<String> {
            let alloc = Allocator::default();
            let mut prog = Parser::new(&alloc, code, SourceType::mjs()).parse().program;
            let guards = super::guards(&alloc, &mut prog);
            let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
            assert!(guards.specifier_exports.is_subset(&guards.symbols));
            guards
                .specifier_exports
                .iter()
                .map(|sym| scoping.symbol_name(*sym).to_string())
                .collect()
        };
        let code = "export const K = 1; let q, r, s; export { q, r as R, K as K2 };";
        assert_eq!(
            specifier_exports(code),
            FxHashSet::from_iter(["q".to_string(), "r".to_string()])
        );
        // A top-level eval can read every binding by name.
        assert!(specifier_exports(&format!("{code} function f(c) {{ eval(c) }}")).is_empty());
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
    fn flags_a_default_specifier_local_only_when_also_exported_by_name() {
        let names = guarded_names("let n = 1, m = 2; export { n as default, m as default2 };");
        assert!(!names.contains("n"), "`n as default` exposes only `default`: {names:?}");
        assert!(names.contains("m"), "{names:?}");
        let names = guarded_names("let n = 1; export { n as default, n };");
        assert!(names.contains("n"), "{names:?}");
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
        let code = "function g() { { function f() {} } { function f() {} } if (g) function h() {} { async function a() {} function* b() {} } }";
        let names = guarded_names_as(code, SourceType::cjs());
        assert!(names.contains("f") && names.contains("h"), "{names:?}");
        assert!(!names.contains("a") && !names.contains("b"), "block-scoped: {names:?}");
        assert!(!names.contains("g"), "{names:?}");
        let strict = guarded_names_as(code, SourceType::mjs());
        assert!(
            !strict.contains("f"),
            "strict block functions are block-scoped: {strict:?}"
        );
    }

    #[test]
    fn annex_b_functions_bound_once_may_be_renamed_on_request() {
        let code = "function g() { { function f() {} } return f; } function h() { var k; { function k() {} } }";
        let allocator = Allocator::default();
        let mut program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut collector = RenameGuardCollector::new().renaming_annex_b_functions();
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
        let scoping = ctx.into_scoping();
        let blocks: Vec<&str> = collector
            .annex_b_blocks()
            .keys()
            .map(|&s| scoping.symbol_name(s))
            .collect();
        assert_eq!(blocks, ["f"]);
        let guarded = collector.into_guarded(&scoping);
        let guarded_named = |name: &str| {
            scoping
                .symbol_ids()
                .filter(|s| scoping.symbol_name(*s) == name && guarded.contains(s))
                .count()
        };
        assert_eq!(guarded_named("f"), 0);
        assert_eq!(guarded_named("k"), 2, "the block function and the `var` it assigns");
    }
}
