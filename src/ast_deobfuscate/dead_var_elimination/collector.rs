use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, ExportDefaultDeclaration, ExportDefaultDeclarationKind,
    ExportNamedDeclaration, FormalParameter, IdentifierReference, ModuleExportName, SimpleAssignmentTarget,
    UpdateExpression, VariableDeclarator,
};
use oxc_traverse::Traverse;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::types::VarInfo;

pub struct DeadVarCollector {
    variables: FxHashMap<String, VarInfo>,
    in_lhs_of_assignment: bool,
    in_declaration: bool,
    current_declaration_name: Option<String>,
}

impl DeadVarCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            variables: FxHashMap::default(),
            in_lhs_of_assignment: false,
            in_declaration: false,
            current_declaration_name: None,
        }
    }

    #[must_use]
    pub fn get_dead_vars(&self) -> FxHashSet<String> {
        self.variables
            .iter()
            .filter(|(_, info)| info.is_dead())
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn record_declaration(&mut self, name: &str) {
        let entry = self.variables.entry(name.to_string()).or_insert_with(VarInfo::new);
        entry.declaration_count += 1;
    }

    fn record_read(&mut self, name: &str) {
        let entry = self.variables.entry(name.to_string()).or_insert_with(VarInfo::new);
        entry.read_count += 1;
    }

    fn record_write(&mut self, name: &str) {
        let entry = self.variables.entry(name.to_string()).or_insert_with(VarInfo::new);
        entry.write_count += 1;
    }

    fn record_function_param(&mut self, name: &str) {
        let entry = self.variables.entry(name.to_string()).or_insert_with(VarInfo::new);
        entry.is_function_param = true;
    }

    fn record_export(&mut self, name: &str) {
        let entry = self.variables.entry(name.to_string()).or_insert_with(VarInfo::new);
        entry.is_exported = true;
    }
}

impl Default for DeadVarCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DeadVarCollector {
    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(ident) = &decl.id {
            let name = ident.name.as_str();
            self.record_declaration(name);
            self.current_declaration_name = Some(name.to_string());
            self.in_declaration = true;
            if crate::ast_deobfuscate::ast_trace_enabled() {
                eprintln!("[AST] Found declaration: {name}");
            }
        }
    }

    fn exit_variable_declarator(&mut self, _decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        self.in_declaration = false;
        self.current_declaration_name = None;
    }

    fn enter_formal_parameter(&mut self, param: &mut FormalParameter<'a>, _ctx: &mut Ctx<'a>) {
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let name = ident.name.as_str();
            self.record_function_param(name);
            if crate::ast_deobfuscate::ast_trace_enabled() {
                eprintln!("[AST] Found function param: {name}");
            }
        }
    }

    fn enter_assignment_expression(&mut self, _expr: &mut AssignmentExpression<'a>, _ctx: &mut Ctx<'a>) {
        self.in_lhs_of_assignment = true;
    }

    fn exit_assignment_expression(&mut self, expr: &mut AssignmentExpression<'a>, _ctx: &mut Ctx<'a>) {
        self.in_lhs_of_assignment = false;

        if let AssignmentTarget::AssignmentTargetIdentifier(ident) = &expr.left {
            let name = ident.name.as_str();
            self.record_write(name);
        }
    }

    fn enter_update_expression(&mut self, expr: &mut UpdateExpression<'a>, _ctx: &mut Ctx<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(ident) = &expr.argument {
            let name = ident.name.as_str();
            self.record_read(name);
            self.record_write(name);
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, _ctx: &mut Ctx<'a>) {
        let name = ident.name.as_str();

        if self.in_declaration
            && let Some(ref decl_name) = self.current_declaration_name
            && name == decl_name
        {
            return;
        }

        if !self.in_lhs_of_assignment {
            self.record_read(name);
        }
    }

    fn enter_export_named_declaration(&mut self, decl: &mut ExportNamedDeclaration<'a>, _ctx: &mut Ctx<'a>) {
        for specifier in &decl.specifiers {
            if let ModuleExportName::IdentifierName(ident) = &specifier.local {
                self.record_export(ident.name.as_str());
            } else if let ModuleExportName::IdentifierReference(ident) = &specifier.local {
                self.record_export(ident.name.as_str());
            }
        }
    }

    fn enter_export_default_declaration(&mut self, decl: &mut ExportDefaultDeclaration<'a>, _ctx: &mut Ctx<'a>) {
        match &decl.declaration {
            ExportDefaultDeclarationKind::Identifier(ident) => {
                self.record_export(ident.name.as_str());
            }
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                if let Some(ref id) = func.id {
                    self.record_export(id.name.as_str());
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(ref id) = class.id {
                    self.record_export(id.name.as_str());
                }
            }
            _ => {}
        }
    }
}
