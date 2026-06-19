use oxc_ast::ast::{Expression, ObjectPropertyKind, Statement};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::names::is_generic_name;
use super::{Ctx, ExportNameMap};

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
    pub const fn map(&self) -> &ExportNameMap {
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
            self.name_map.entry(symbol_id).or_insert_with(|| original.to_string());
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
