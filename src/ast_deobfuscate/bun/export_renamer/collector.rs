use oxc_ast::ast::{CallExpression, Expression, ObjectPropertyKind};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::names::is_generic_name;
use super::{Ctx, ExportNameMap};

/// Collects mangled→original mappings from calls to the `__export` runtime
/// helper, whichever name it has.
pub struct ExportNameCollector {
    /// The bindings of the detected `__export` helpers.
    export_helpers: FxHashSet<SymbolId>,
    /// Map from the resolved exported symbol → original export name.
    name_map: ExportNameMap,
}

impl ExportNameCollector {
    #[must_use]
    pub fn new(export_helpers: FxHashSet<SymbolId>) -> Self {
        Self {
            export_helpers,
            name_map: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> ExportNameMap {
        self.name_map
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

impl<'a> Traverse<'a, DeobfuscateState> for ExportNameCollector {
    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, ctx: &mut Ctx<'a>) {
        self.try_extract_from_call(call, ctx);
    }
}
