//! Bun __export() name extraction and renaming pass
//!
//! Extracts original export names from `__export(target, { originalName: () => mangledName })`
//! calls and renames `mangledName` → `originalName` throughout the program.
//!
//! This is the AST-based replacement for the regex approach in `bun_extract.rs`.
//! It fires during the deobfuscation pipeline (Phase 0.95) so that all subsequent
//! passes see the readable names.
//!
//! Example:
//! ```js
//! __export(GZ8, {
//!     writeToStdout: () => UK,
//!     exitWithError: () => Ba4
//! });
//! function UK(data) { process.stdout.write(data); }
//! ```
//! becomes:
//! ```js
//! __export(GZ8, {
//!     writeToStdout: () => writeToStdout,
//!     exitWithError: () => exitWithError
//! });
//! function writeToStdout(data) { process.stdout.write(data); }
//! ```

use oxc_ast::ast::{Expression, IdentifierReference, ObjectPropertyKind, Statement};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type ExportNameMap = FxHashMap<SymbolId, String>;

/// Collects mangled→original mappings from `__export()` / `j$()` / `MR()` calls.
pub struct ExportNameCollector {
    /// Map from the resolved exported symbol → original export name.
    name_map: ExportNameMap,
}

impl ExportNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name_map: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> ExportNameMap {
        self.name_map
    }

    #[must_use]
    pub fn map(&self) -> &ExportNameMap {
        &self.name_map
    }

    /// Check if a call expression is `__export(target, { ... })` or similar.
    fn try_extract_from_call(&mut self, expr: &Expression<'_>, ctx: &Ctx<'_>) {
        let Expression::CallExpression(call) = expr else { return };

        // Callee must be an identifier named __export, j$, or MR
        let callee_name = match &call.callee {
            Expression::Identifier(id) => id.name.as_str(),
            _ => return,
        };

        if !matches!(callee_name, "__export" | "j$" | "MR" | "__exportValue") {
            return;
        }

        // Must have exactly 2 arguments
        if call.arguments.len() != 2 {
            return;
        }

        // Second argument must be an object expression
        let Some(second) = call.arguments.get(1) else { return };
        let obj_expr = match second {
            oxc_ast::ast::Argument::ObjectExpression(obj) => obj,
            _ => return,
        };

        // Each property: `originalName: () => mangledName`
        for prop in &obj_expr.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else {
                continue;
            };

            // Key is the original name
            let Some(original_name) = p.key.name() else { continue };

            // Value is `() => mangledName` (arrow returning an identifier)
            let Expression::ArrowFunctionExpression(arrow) = &p.value else {
                continue;
            };

            if !arrow.expression || !arrow.params.items.is_empty() {
                continue;
            }

            // Body should be a single ExpressionStatement with an Identifier
            let Some(stmt) = arrow.body.statements.first() else {
                continue;
            };
            let Statement::ExpressionStatement(es) = stmt else {
                continue;
            };
            let Expression::Identifier(mangled_id) = &es.expression else {
                continue;
            };

            let mangled = mangled_id.name.as_str();
            let original = original_name.as_ref();
            let Some(symbol_id) = ctx.scoping().get_reference(mangled_id.reference_id()).symbol_id() else {
                continue;
            };

            // Skip if original == mangled (already named)
            if mangled == original {
                continue;
            }

            // Skip generic names that aren't helpful
            if is_generic_name(original) {
                continue;
            }

            // Don't overwrite existing mappings
            if !self.name_map.contains_key(&symbol_id) {
                self.name_map.insert(symbol_id, original.to_string());
            }
        }
    }
}

impl Default for ExportNameCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameCollector {
    fn enter_expression_statement(&mut self, es: &mut oxc_ast::ast::ExpressionStatement<'a>, ctx: &mut Ctx<'a>) {
        self.try_extract_from_call(&es.expression, ctx);
    }
}

/// Renames exported identifiers by resolved symbol, not by raw minified text.
pub struct ExportNameRenamer {
    name_map: ExportNameMap,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl ExportNameRenamer {
    #[must_use]
    pub fn new(name_map: ExportNameMap) -> Self {
        Self {
            name_map,
            renamed_refs: 0,
            renamed_decls: 0,
        }
    }

    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }

    #[must_use]
    pub const fn renamed_decls(&self) -> usize {
        self.renamed_decls
    }

    #[must_use]
    pub fn total_renamed(&self) -> usize {
        self.renamed_refs + self.renamed_decls
    }

    fn lookup(&self, symbol_id: SymbolId) -> Option<&str> {
        self.name_map.get(&symbol_id).map(String::as_str)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = ctx.ast.ident(original);
            self.renamed_refs += 1;
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut oxc_ast::ast::BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ident.symbol_id.get() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = ctx.ast.ident(original);
            self.renamed_decls += 1;
        }
    }

    fn exit_function(&mut self, func: &mut oxc_ast::ast::Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id {
            let Some(symbol_id) = id.symbol_id.get() else {
                return;
            };
            if let Some(original) = self.lookup(symbol_id)
                && id.name.as_str() != original
            {
                id.name = ctx.ast.ident(original);
                self.renamed_decls += 1;
            }
        }
    }
}

/// JS reserved words + dangerous globals that must never become a binding name
/// (renaming a symbol to one of these produces invalid JavaScript, e.g.
/// `var instanceof = ...` from a Zod `z.instanceof` export key).
fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "await"
            | "async"
            | "undefined"
            | "arguments"
            | "eval"
            | "NaN"
            | "Infinity"
            | "globalThis"
    )
}

/// Make the collected rename map safe to apply.
///
/// Bun reuses the same minified name across non-overlapping scopes, and many
/// modules export the same public name (`ZodType`, `Client`, `Command`, …).
/// Applying the raw collected map can therefore (a) assign a JS reserved word
/// (`instanceof`) or (b) give two distinct symbols that share a scope the same
/// name — both produce invalid JavaScript (redeclaration / parse errors seen on
/// the real cli.js bundle). This finalizer guarantees every assigned name is a
/// legal identifier and unique within its declaring scope (vs. the other
/// renamed symbols *and* the pre-existing bindings in that scope), suffixing
/// with `$N` on collision.
#[must_use]
pub fn make_names_unique(map: ExportNameMap, scoping: &Scoping) -> ExportNameMap {
    if map.is_empty() {
        return map;
    }

    // Seed each scope's in-use name set with the bindings that are NOT being
    // renamed away, so a rename can't collide with a pre-existing identifier.
    let renamed: FxHashSet<SymbolId> = map.keys().copied().collect();
    let mut used: FxHashMap<ScopeId, FxHashSet<String>> = FxHashMap::default();
    for sym in scoping.symbol_ids() {
        if renamed.contains(&sym) {
            continue;
        }
        let scope = scoping.symbol_scope_id(sym);
        used.entry(scope)
            .or_default()
            .insert(scoping.symbol_name(sym).to_string());
    }

    // Deterministic assignment order → stable output across runs / versions.
    let mut entries: Vec<(SymbolId, String)> = map.into_iter().collect();
    entries.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });

    let mut out: ExportNameMap = FxHashMap::default();
    for (sym, desired) in entries {
        let scope = scoping.symbol_scope_id(sym);
        let base = if is_reserved_word(&desired) {
            format!("{desired}$")
        } else {
            desired
        };
        let set = used.entry(scope).or_default();
        let mut candidate = base.clone();
        let mut n = 1u32;
        while set.contains(&candidate) {
            candidate = format!("{base}${n}");
            n += 1;
        }
        set.insert(candidate.clone());
        out.insert(sym, candidate);
    }
    out
}

/// Names that are too generic to be useful when applied as renames.
fn is_generic_name(name: &str) -> bool {
    matches!(
        name,
        "default"
            | "init"
            | "get"
            | "set"
            | "call"
            | "apply"
            | "bind"
            | "then"
            | "catch"
            | "finally"
            | "next"
            | "done"
            | "value"
            | "run"
            | "exec"
            | "start"
            | "stop"
            | "end"
            | "new"
            | "create"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn collect_names(code: &str) -> ExportNameMap {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;
        let mut collector = ExportNameCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        collector.into_map()
    }

    fn collect_and_rename(code: &str) -> (String, usize) {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;

        let mut collector = ExportNameCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        let map = collector.into_map();

        let mut renamer = ExportNameRenamer::new(map);
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);

        (Codegen::new().build(&prog).code, renamer.total_renamed())
    }

    #[test]
    fn extracts_export_names() {
        let code = r#"
            var UK, Ba4;
            __export(target, {
                writeToStdout: () => UK,
                exitWithError: () => Ba4
            });
        "#;
        let map = collect_names(code);
        assert!(map.values().any(|name| name == "writeToStdout"));
        assert!(map.values().any(|name| name == "exitWithError"));
    }

    #[test]
    fn skips_already_named() {
        let code = r#"
            __export(target, {
                foo: () => foo,
                bar: () => bar
            });
        "#;
        let map = collect_names(code);
        assert!(map.is_empty(), "Should skip when mangled == original");
    }

    #[test]
    fn skips_generic_names() {
        let code = r#"
            __export(target, { default: () => XYZ });
        "#;
        let map = collect_names(code);
        assert!(map.is_empty(), "Should skip 'default'");
    }

    #[test]
    fn renames_throughout_program() {
        let code = r#"
            __export(target, { writeToStdout: () => UK });
            function UK(data) { process.stdout.write(data); }
            UK("hello");
        "#;
        let (out, count) = collect_and_rename(code);
        assert!(count >= 3, "Should rename decl + refs, got {count}");
        assert!(
            out.contains("function writeToStdout("),
            "Should rename function decl: {out}"
        );
        assert!(out.contains("writeToStdout(\"hello\")"), "Should rename call: {out}");
    }

    #[test]
    fn handles_j_dollar_form() {
        let code = r#"
            j$(target, { fetchData: () => abc });
            var abc = async () => {};
        "#;
        let map = collect_names(code);
        assert!(map.values().any(|name| name == "fetchData"));
    }

    #[test]
    fn handles_large_export_block() {
        let code = r#"
            var Fq, XB, Y0, Sg, pE;
            __export(D6, {
                string: () => Fq,
                number: () => XB,
                boolean: () => Y0,
                array: () => Sg,
                object: () => pE
            });
        "#;
        let map = collect_names(code);
        assert_eq!(map.len(), 5);
        assert!(map.values().any(|name| name == "string"));
        assert!(map.values().any(|name| name == "number"));
        assert!(map.values().any(|name| name == "boolean"));
    }

    #[test]
    fn only_renames_resolved_export_symbol() {
        let code = r#"
            function exported(c) { return c; }
            __export(target, { logEvent: () => exported });
            function unrelated(c) { return c + 1; }
            exported(1);
            unrelated(2);
        "#;
        let (out, _count) = collect_and_rename(code);
        assert!(
            out.contains("function logEvent(c)"),
            "exported function should rename: {out}"
        );
        assert!(out.contains("logEvent(1)"), "exported call should rename: {out}");
        assert!(
            out.contains("function unrelated(c)"),
            "unrelated parameter must stay c: {out}"
        );
        assert!(
            out.contains("return c + 1") || out.contains("return c+1"),
            "unrelated body must stay c: {out}"
        );
    }
}
