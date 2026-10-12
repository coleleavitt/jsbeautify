use oxc_ast::ast::{
    CallExpression, ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExportNamedDeclaration, Expression,
    ModuleExportName, ObjectPropertyKind, Program,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::default_export::{DefaultExport, default_export_names, is_minified};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::names::{is_generic_name, is_reserved_word, is_valid_identifier};
use super::{Ctx, ExportNameMap};

/// Collects mangled→original mappings from calls to the `__export` runtime
/// helper, whichever name it has, and from native `export { a as Button }`
/// specifiers, and names the locals behind `default` exports, of `__export`
/// calls and of native `export default`.
pub struct ExportNameCollector {
    /// The bindings of the detected `__export` helpers.
    export_helpers: FxHashSet<SymbolId>,
    /// Map from the resolved exported symbol → original export name.
    name_map: ExportNameMap,
    /// Minified locals of `export { local as name }` → their export name.
    specifier_names: ExportNameMap,
    defaults: Vec<DefaultExport>,
    /// esbuild's `// src/file.ts` module comments: (start offset, path).
    module_comments: Vec<(u32, String)>,
    /// `__export` calls with a `default` key so far, which number the modules
    /// that have no module comment.
    default_modules: usize,
}

impl ExportNameCollector {
    #[must_use]
    pub fn new(export_helpers: FxHashSet<SymbolId>) -> Self {
        Self {
            export_helpers,
            name_map: FxHashMap::default(),
            specifier_names: FxHashMap::default(),
            defaults: Vec::new(),
            module_comments: Vec::new(),
            default_modules: 0,
        }
    }

    /// The path of the last esbuild module comment before `offset`.
    fn module_path(&self, offset: u32) -> Option<String> {
        let index = self.module_comments.partition_point(|(start, _)| *start < offset);
        index.checked_sub(1).map(|i| self.module_comments[i].1.clone())
    }

    fn push_default(&mut self, local: SymbolId, module: Option<String>) {
        self.defaults.push(DefaultExport { local, module });
    }

    #[must_use]
    pub fn into_map(self) -> ExportNameMap {
        self.name_map
    }

    /// The names from `__export` calls and default exports, and separately
    /// the export names of minified `export { local as name }` specifier
    /// locals. Renaming such a local to its export name keeps the public name
    /// (`export { Button }`), but the rename guards protect it, so the caller
    /// applies these only to [`RenameGuards::specifier_exports`].
    ///
    /// [`RenameGuards::specifier_exports`]: crate::ast_deobfuscate::rename_guards::RenameGuards::specifier_exports
    #[must_use]
    pub fn into_maps(self) -> (ExportNameMap, ExportNameMap) {
        (self.name_map, self.specifier_names)
    }

    #[must_use]
    pub const fn map(&self) -> &ExportNameMap {
        &self.name_map
    }

    /// Records the names of an `__export(target, { name: () => local })` call.
    fn try_extract_from_call(&mut self, call: &CallExpression<'_>, ctx: &Ctx<'_>) {
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let callee_symbol = ctx.scoping().get_reference(callee.reference_id()).symbol_id();
        if !callee_symbol.is_some_and(|symbol| self.export_helpers.contains(&symbol)) {
            return;
        }

        // Must have exactly 2 arguments
        if call.arguments.len() != 2 {
            return;
        }

        // Second argument must be an object expression
        let Some(oxc_ast::ast::Argument::ObjectExpression(obj_expr)) = call.arguments.get(1) else {
            return;
        };

        // Each property: `originalName: () => mangledName`
        let mut module = None;
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

            if !arrow.params.items.is_empty() {
                continue;
            }

            let Some(Expression::Identifier(mangled_id)) = arrow.body.as_expression() else {
                continue;
            };

            let mangled = mangled_id.name.as_str();
            let original = original_name.as_ref();
            let Some(symbol_id) = ctx.scoping().get_reference(mangled_id.reference_id()).symbol_id() else {
                continue;
            };

            if original == "default" {
                let module = module
                    .get_or_insert_with(|| {
                        self.module_path(call.span.start).unwrap_or_else(|| {
                            self.default_modules += 1;
                            self.default_modules.to_string()
                        })
                    })
                    .clone();
                self.push_default(symbol_id, Some(module));
                continue;
            }

            // Skip if original == mangled (already named)
            if mangled == original {
                continue;
            }

            // Skip generic names that aren't helpful
            if is_generic_name(original) {
                continue;
            }

            // Don't overwrite existing mappings
            self.name_map.entry(symbol_id).or_insert_with(|| original.to_string());
        }
    }
}

/// Whether an export name can become its minified local's name: an identifier
/// that is not a reserved word and longer than the one or two characters
/// rollup and esbuild give the exports code-split chunks share
/// (`export { r as a }` says nothing about `r`).
fn is_informative_local_name(name: &str) -> bool {
    is_valid_identifier(name) && !is_reserved_word(name) && name.chars().count() > 2
}

/// `src/components/Button.tsx`: the text of an esbuild module comment.
fn is_module_path(text: &str) -> bool {
    const EXTENSIONS: [&str; 12] = [
        "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "json", "css", "vue", "svelte",
    ];
    !text.is_empty()
        && !text.contains(char::is_whitespace)
        && text
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && EXTENSIONS.contains(&ext))
}

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        self.module_comments = program
            .comments
            .iter()
            .filter(|comment| comment.is_line())
            .filter_map(|comment| {
                let text = comment.content_span().source_text(program.source_text).trim();
                is_module_path(text).then(|| (comment.span.start, text.to_string()))
            })
            .collect();
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let defaults = std::mem::take(&mut self.defaults);
        for (symbol, name) in default_export_names(program, ctx.scoping(), defaults) {
            self.name_map.entry(symbol).or_insert(name);
        }
    }

    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        self.try_extract_from_call(call, ctx);
    }

    /// `export { local as default }`, `export { local as name }`
    fn enter_export_named_declaration(&mut self, export: &mut ExportNamedDeclaration<'a>, ctx: &mut Ctx<'a>) {
        for spec in &export.specifiers {
            let ModuleExportName::IdentifierReference(local) = &spec.local else {
                continue;
            };
            let Some(symbol) = ctx.scoping().get_reference(local.reference_id()).symbol_id() else {
                continue;
            };
            let exported = spec.exported.name();
            if exported == "default" {
                self.push_default(symbol, None);
            } else if is_minified(&local.name) && is_informative_local_name(&exported) {
                self.specifier_names
                    .entry(symbol)
                    .or_insert_with(|| exported.to_string());
            }
        }
    }

    /// `export default local`, `export default function local() {}`, `export default class local {}`
    fn enter_export_default_declaration(&mut self, export: &mut ExportDefaultDeclaration<'a>, ctx: &mut Ctx<'a>) {
        let symbol = match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                func.id.as_ref().and_then(|id| id.symbol_id.get())
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                class.id.as_ref().and_then(|id| id.symbol_id.get())
            }
            ExportDefaultDeclarationKind::Identifier(id) => ctx.scoping().get_reference(id.reference_id()).symbol_id(),
            _ => None,
        };
        if let Some(symbol) = symbol {
            self.push_default(symbol, None);
        }
    }
}
