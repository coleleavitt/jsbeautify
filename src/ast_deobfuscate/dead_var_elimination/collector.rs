use oxc_ast::ast::{
    BindingPattern, CallExpression, ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExportNamedDeclaration,
    Expression, IdentifierReference, ModuleExportName, VariableDeclarator,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::{UnresolvedReference, VarInfo};

pub struct DeadVarCollector {
    variables: FxHashMap<SymbolId, VarInfo>,
    unresolved_references: FxHashSet<UnresolvedReference>,
    has_direct_eval_syntax: bool,
    resolution_failed: bool,
}

impl DeadVarCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            variables: FxHashMap::default(),
            unresolved_references: FxHashSet::default(),
            has_direct_eval_syntax: false,
            resolution_failed: false,
        }
    }

    #[must_use]
    pub fn get_dead_vars(&self) -> FxHashSet<SymbolId> {
        if self.resolution_failed || self.has_direct_eval_syntax {
            return FxHashSet::default();
        }

        self.variables
            .iter()
            .filter(|(_, info)| info.is_dead())
            .map(|(&symbol_id, _)| symbol_id)
            .collect()
    }

    pub(super) const fn unresolved_references(&self) -> &FxHashSet<UnresolvedReference> {
        &self.unresolved_references
    }

    pub(super) const fn resolution_failed(&self) -> bool {
        self.resolution_failed
    }

    fn record_declaration(&mut self, symbol_id: SymbolId) {
        let entry = self.variables.entry(symbol_id).or_insert_with(VarInfo::new);
        entry.declaration_count += 1;
    }

    fn record_read(&mut self, symbol_id: SymbolId) {
        let entry = self.variables.entry(symbol_id).or_insert_with(VarInfo::new);
        entry.read_count += 1;
    }

    fn record_write(&mut self, symbol_id: SymbolId) {
        let entry = self.variables.entry(symbol_id).or_insert_with(VarInfo::new);
        entry.write_count += 1;
    }

    fn record_export(&mut self, symbol_id: SymbolId) {
        let entry = self.variables.entry(symbol_id).or_insert_with(VarInfo::new);
        entry.is_exported = true;
    }

    fn record_export_reference(&mut self, ident: &IdentifierReference<'_>, ctx: &Ctx<'_>) {
        let Some(reference_id) = ident.reference_id.get() else {
            self.resolution_failed = true;
            return;
        };
        let Some(symbol_id) = ctx.scoping().get_reference(reference_id).symbol_id() else {
            self.resolution_failed = true;
            return;
        };
        self.record_export(symbol_id);
    }
}

impl Default for DeadVarCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeadVarCollector {
    fn enter_call_expression(&mut self, call: &mut CallExpression<'a>, _ctx: &mut Ctx<'a>) {
        if matches!(&call.callee, Expression::Identifier(id) if id.name == "eval") {
            self.has_direct_eval_syntax = true;
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(ident) = &decl.id else {
            return;
        };
        let Some(symbol_id) = ident.symbol_id.get() else {
            self.resolution_failed = true;
            return;
        };

        self.record_declaration(symbol_id);
        if crate::ast_deobfuscate::ast_trace_enabled() {
            eprintln!("[AST] Found declaration: {}", ident.name);
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(reference_id) = ident.reference_id.get() else {
            self.resolution_failed = true;
            return;
        };
        let reference = ctx.scoping().get_reference(reference_id);
        let Some(symbol_id) = reference.symbol_id() else {
            self.unresolved_references.insert(UnresolvedReference {
                name: ident.name.to_string(),
                span_start: ident.span.start,
                span_end: ident.span.end,
            });
            return;
        };

        if reference.is_read() {
            self.record_read(symbol_id);
        }
        if reference.is_write() {
            self.record_write(symbol_id);
        }
    }

    fn enter_export_named_declaration(&mut self, decl: &mut ExportNamedDeclaration<'a>, ctx: &mut Ctx<'a>) {
        for specifier in &decl.specifiers {
            match &specifier.local {
                ModuleExportName::IdentifierReference(ident) => self.record_export_reference(ident, ctx),
                ModuleExportName::IdentifierName(_) => self.resolution_failed = true,
                ModuleExportName::StringLiteral(_) => {}
            }
        }
    }

    fn enter_export_default_declaration(&mut self, decl: &mut ExportDefaultDeclaration<'a>, ctx: &mut Ctx<'a>) {
        if let ExportDefaultDeclarationKind::Identifier(ident) = &decl.declaration {
            self.record_export_reference(ident, ctx);
        }
    }
}
