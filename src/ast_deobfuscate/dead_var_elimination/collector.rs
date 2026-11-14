use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, CallExpression, Declaration, ExportDeclaration, ExportDefaultDeclaration,
    ExportDefaultDeclarationKind, ExportNamedDeclaration, Expression, FormalParameters, FunctionBody,
    IdentifierReference, ModuleExportName, Program, Statement, VariableDeclarator, WithStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::annex_b::annex_b_shadowed_symbols;
use crate::ast_deobfuscate::rename_guards::is_global_binding;

use super::eliminator::{is_proven_safe_initializer, removable_function_symbol};
use super::types::UnresolvedReference;

/// Finds the declared bindings the program can never read.
///
/// A reference inside a declaration the eliminator can remove (a statement
/// `var`/`let`/`const` declarator with a side-effect-free initializer, or a
/// function declaration directly in a program or function body) only runs if
/// that binding is live, so it is an edge from the binding rather than a use.
/// Every other reference, read or write, keeps its binding live. Bindings
/// unreachable from those uses are dead; this covers a decoder function that
/// is the only reader of its string array, and a function that reassigns
/// itself from its own body.
///
/// Exports, classic-script globals and parameters (which `arguments` aliases
/// in sloppy mode) are always live, and so is every binding an Annex B block
/// function may replace ([`annex_b_shadowed_symbols`]): references resolved to
/// one binding may read another at runtime.
/// Direct `eval` syntax or a `with` statement anywhere disables elimination.
pub struct DeadVarCollector {
    declared: FxHashSet<SymbolId>,
    removable: FxHashSet<SymbolId>,
    live: FxHashSet<SymbolId>,
    unresolved_references: FxHashSet<UnresolvedReference>,
    has_dynamic_scope: bool,
    resolution_failed: bool,
}

impl DeadVarCollector {
    #[must_use]
    pub fn collect(program: &Program<'_>, scoping: &Scoping) -> Self {
        let mut visitor = Collector {
            scoping,
            source_type: program.source_type,
            owner: None,
            declared: FxHashSet::default(),
            removable: FxHashSet::default(),
            uses: Vec::new(),
            edges: FxHashMap::default(),
            unresolved_references: FxHashSet::default(),
            has_dynamic_scope: false,
            resolution_failed: false,
        };
        visitor.visit_program(program);
        visitor.uses.extend(annex_b_shadowed_symbols(program, scoping));

        let mut live: FxHashSet<SymbolId> = visitor.uses.into_iter().collect();
        let mut pending: Vec<SymbolId> = live.iter().copied().collect();
        while let Some(symbol) = pending.pop() {
            for &target in visitor.edges.get(&symbol).into_iter().flatten() {
                if live.insert(target) {
                    pending.push(target);
                }
            }
        }

        Self {
            declared: visitor.declared,
            removable: visitor.removable,
            live,
            unresolved_references: visitor.unresolved_references,
            has_dynamic_scope: visitor.has_dynamic_scope,
            resolution_failed: visitor.resolution_failed,
        }
    }

    #[must_use]
    pub fn get_dead_vars(&self) -> FxHashSet<SymbolId> {
        if self.resolution_failed || self.has_dynamic_scope {
            return FxHashSet::default();
        }

        self.declared.difference(&self.live).copied().collect()
    }

    /// Whether the eliminator would remove a declaration of one of `dead_vars`.
    pub(super) fn removes_any(&self, dead_vars: &FxHashSet<SymbolId>) -> bool {
        !self.removable.is_disjoint(dead_vars)
    }

    pub(super) const fn unresolved_references(&self) -> &FxHashSet<UnresolvedReference> {
        &self.unresolved_references
    }

    pub(super) const fn resolution_failed(&self) -> bool {
        self.resolution_failed
    }
}

struct Collector<'s> {
    scoping: &'s Scoping,
    source_type: SourceType,
    /// The innermost removable declaration being visited.
    owner: Option<SymbolId>,
    declared: FxHashSet<SymbolId>,
    removable: FxHashSet<SymbolId>,
    /// Bindings referenced outside every removable declaration, or pinned.
    uses: Vec<SymbolId>,
    edges: FxHashMap<SymbolId, Vec<SymbolId>>,
    unresolved_references: FxHashSet<UnresolvedReference>,
    has_dynamic_scope: bool,
    resolution_failed: bool,
}

impl Collector<'_> {
    /// Statements of a program or function body, where a function declaration
    /// is never an Annex B block function.
    fn visit_body(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            if let Statement::FunctionDeclaration(func) = statement
                && let Some(symbol) = removable_function_symbol(func)
            {
                self.declare(symbol);
                self.removable.insert(symbol);
                let outer = self.owner.replace(symbol);
                walk::walk_statement(self, statement);
                self.owner = outer;
            } else {
                self.visit_statement(statement);
            }
        }
    }

    fn visit_declarator(&mut self, decl: &VariableDeclarator<'_>, in_statement: bool) {
        let mut owner = None;
        if let BindingPattern::BindingIdentifier(ident) = &decl.id {
            let Some(symbol) = ident.symbol_id.get() else {
                self.resolution_failed = true;
                return;
            };
            self.declare(symbol);
            if in_statement && decl.init.as_ref().is_none_or(is_proven_safe_initializer) {
                self.removable.insert(symbol);
                owner = Some(symbol);
            }
        }
        let outer = self.owner;
        self.owner = owner.or(outer);
        walk::walk_variable_declarator(self, decl);
        self.owner = outer;
    }

    fn declare(&mut self, symbol: SymbolId) {
        self.declared.insert(symbol);
        if is_global_binding(self.source_type, self.scoping, symbol) {
            self.uses.push(symbol);
        }
    }

    fn pin_binding(&mut self, ident: &BindingIdentifier<'_>) {
        match ident.symbol_id.get() {
            Some(symbol) => self.uses.push(symbol),
            None => self.resolution_failed = true,
        }
    }

    fn pin_reference(&mut self, ident: &IdentifierReference<'_>) {
        match ident
            .reference_id
            .get()
            .and_then(|reference| self.scoping.get_reference(reference).symbol_id())
        {
            Some(symbol) => self.uses.push(symbol),
            None => self.resolution_failed = true,
        }
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.visit_body(&program.body);
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.visit_body(&body.statements);
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::VariableDeclaration(decl) => {
                for declarator in &decl.declarations {
                    self.visit_declarator(declarator, true);
                }
            }
            _ => walk::walk_statement(self, statement),
        }
    }

    /// Declarators outside a declaration statement: `for` heads and exports.
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        self.visit_declarator(decl, false);
    }

    fn visit_formal_parameters(&mut self, params: &FormalParameters<'a>) {
        let rest = params.rest.iter().map(|rest| &rest.rest.argument);
        for pattern in params.items.iter().map(|param| &param.pattern).chain(rest) {
            for ident in pattern.get_binding_identifiers() {
                self.pin_binding(ident);
            }
        }
        walk::walk_formal_parameters(self, params);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if matches!(call.callee.without_parentheses(), Expression::Identifier(id) if id.name == "eval") {
            self.has_dynamic_scope = true;
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_with_statement(&mut self, stmt: &WithStatement<'a>) {
        self.has_dynamic_scope = true;
        walk::walk_with_statement(self, stmt);
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        let Some(reference_id) = ident.reference_id.get() else {
            self.resolution_failed = true;
            return;
        };
        let Some(symbol) = self.scoping.get_reference(reference_id).symbol_id() else {
            self.unresolved_references.insert(UnresolvedReference {
                name: ident.name.to_string(),
                span_start: ident.span.start,
                span_end: ident.span.end,
            });
            return;
        };
        match self.owner {
            Some(owner) if owner != symbol => self.edges.entry(owner).or_default().push(symbol),
            Some(_) => {}
            None => self.uses.push(symbol),
        }
    }

    fn visit_export_named_declaration(&mut self, decl: &ExportNamedDeclaration<'a>) {
        for specifier in &decl.specifiers {
            match &specifier.local {
                ModuleExportName::IdentifierReference(ident) => self.pin_reference(ident),
                ModuleExportName::IdentifierName(_) => self.resolution_failed = true,
                ModuleExportName::StringLiteral(_) => {}
            }
        }
        walk::walk_export_named_declaration(self, decl);
    }

    fn visit_export_declaration(&mut self, decl: &ExportDeclaration<'a>) {
        match &decl.declaration {
            Declaration::VariableDeclaration(var_decl) => {
                for ident in var_decl
                    .declarations
                    .iter()
                    .flat_map(|d| d.id.get_binding_identifiers())
                {
                    self.pin_binding(ident);
                }
            }
            Declaration::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    self.pin_binding(id);
                }
            }
            Declaration::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    self.pin_binding(id);
                }
            }
            _ => {}
        }
        walk::walk_export_declaration(self, decl);
    }

    fn visit_export_default_declaration(&mut self, decl: &ExportDefaultDeclaration<'a>) {
        match &decl.declaration {
            ExportDefaultDeclarationKind::Identifier(ident) => self.pin_reference(ident),
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    self.pin_binding(id);
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    self.pin_binding(id);
                }
            }
            _ => {}
        }
        walk::walk_export_default_declaration(self, decl);
    }
}
