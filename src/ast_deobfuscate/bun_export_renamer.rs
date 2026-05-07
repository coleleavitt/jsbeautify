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

use oxc_ast::ast::{BindingPattern, Expression, IdentifierReference, ObjectPropertyKind, Program, Statement};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Collects mangled→original mappings from `__export()` / `j$()` / `MR()` calls.
pub struct ExportNameCollector {
    /// Map from mangled name → original export name
    name_map: FxHashMap<String, String>,
}

impl ExportNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name_map: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> FxHashMap<String, String> {
        self.name_map
    }

    #[must_use]
    pub fn map(&self) -> &FxHashMap<String, String> {
        &self.name_map
    }

    /// Check if a call expression is `__export(target, { ... })` or similar.
    fn try_extract_from_call(&mut self, expr: &Expression<'_>) {
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

            // Skip if original == mangled (already named)
            if mangled == original {
                continue;
            }

            // Skip generic names that aren't helpful
            if is_generic_name(original) {
                continue;
            }

            // Don't overwrite existing mappings
            if !self.name_map.contains_key(mangled) {
                self.name_map.insert(mangled.to_string(), original.to_string());
            }
        }
    }

    /// Also extract from `this.name = "ClassName"` in constructors.
    fn try_extract_this_name(&mut self, stmts: &[Statement<'_>]) {
        for stmt in stmts {
            if let Statement::ExpressionStatement(es) = stmt {
                if let Expression::AssignmentExpression(assign) = &es.expression {
                    if let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left {
                        if let Expression::ThisExpression(_) = &member.object {
                            if member.property.name.as_str() == "name" {
                                if let Expression::StringLiteral(s) = &assign.right {
                                    let class_name = s.value.as_str();
                                    if !class_name.is_empty() && class_name.len() > 2 {
                                        // We'd need the enclosing class/function name to map,
                                        // which requires more context. Skip for now — handled
                                        // in a separate pass.
                                        let _ = class_name;
                                    }
                                }
                            }
                        }
                    }
                }
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
    fn enter_expression_statement(&mut self, es: &mut oxc_ast::ast::ExpressionStatement<'a>, _ctx: &mut Ctx<'a>) {
        self.try_extract_from_call(&es.expression);
    }
}

/// Renames all identifiers from mangled→original using the collected map.
pub struct ExportNameRenamer {
    name_map: FxHashMap<String, String>,
    renamed_refs: usize,
    renamed_decls: usize,
}

impl ExportNameRenamer {
    #[must_use]
    pub fn new(name_map: FxHashMap<String, String>) -> Self {
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

    fn lookup(&self, mangled: &str) -> Option<&str> {
        self.name_map.get(mangled).map(String::as_str)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(original) = self.lookup(ident.name.as_str()) {
            ident.name = ctx.ast.ident(original);
            self.renamed_refs += 1;
        }
    }

    fn exit_binding_identifier(&mut self, ident: &mut oxc_ast::ast::BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        if let Some(original) = self.lookup(ident.name.as_str()) {
            ident.name = ctx.ast.ident(original);
            self.renamed_decls += 1;
        }
    }

    fn exit_function(&mut self, func: &mut oxc_ast::ast::Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(id) = &mut func.id {
            if let Some(original) = self.lookup(id.name.as_str()) {
                id.name = ctx.ast.ident(original);
                self.renamed_decls += 1;
            }
        }
    }
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

    fn collect_names(code: &str) -> FxHashMap<String, String> {
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
            __export(target, {
                writeToStdout: () => UK,
                exitWithError: () => Ba4
            });
        "#;
        let map = collect_names(code);
        assert_eq!(map.get("UK").map(String::as_str), Some("writeToStdout"));
        assert_eq!(map.get("Ba4").map(String::as_str), Some("exitWithError"));
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
        assert_eq!(map.get("abc").map(String::as_str), Some("fetchData"));
    }

    #[test]
    fn handles_large_export_block() {
        let code = r#"
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
        assert_eq!(map.get("Fq").map(String::as_str), Some("string"));
        assert_eq!(map.get("XB").map(String::as_str), Some("number"));
        assert_eq!(map.get("Y0").map(String::as_str), Some("boolean"));
    }
}
