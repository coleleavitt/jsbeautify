//! Class renamer based on `this.name = "ClassName"` patterns.
//!
//! Bun-minified bundles often have patterns like:
//! ```js
//! c5 = class c5 extends Error {
//!     constructor(H) {
//!         super(H);
//!         this.name = "AbortError";
//!     }
//! };
//! ```
//! This pass detects `this.name = "X"` inside class constructors and
//! renames the class from its mangled name to the real name.
//!
//! Also handles `.displayName = "ComponentName"` for React components.

use oxc_ast::ast::{
    BindingPattern, Class, ClassElement, Expression, FunctionBody, IdentifierReference, MethodDefinitionKind,
    Statement, VariableDeclarator,
};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
pub type ClassNameMap = FxHashMap<SymbolId, String>;

pub struct ClassNameCollector {
    /// Map from resolved class/variable symbol → real name (from this.name)
    name_map: ClassNameMap,
}

impl ClassNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name_map: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> ClassNameMap {
        self.name_map
    }

    #[must_use]
    pub fn map(&self) -> &ClassNameMap {
        &self.name_map
    }

    fn real_name_from_class<'a>(class: &Class<'a>, mangled: &str) -> Option<&'a str> {
        // Skip if already a long readable name
        if mangled.len() > 5 && mangled.chars().any(|c| c.is_ascii_lowercase()) {
            return None;
        }

        // Look for constructor with this.name = "..."
        for element in &class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if method.kind != MethodDefinitionKind::Constructor {
                continue;
            }
            let Some(body) = &method.value.body else { continue };
            if let Some(real_name) = find_this_name_assignment(body) {
                if real_name != mangled && real_name.len() > 2 && !is_xml_name(real_name) {
                    return Some(real_name);
                }
            }
        }
        None
    }

    fn try_extract_from_class(&mut self, class: &Class<'_>) {
        let Some(id) = &class.id else { return };
        let Some(symbol_id) = id.symbol_id.get() else { return };
        let Some(real_name) = Self::real_name_from_class(class, id.name.as_str()) else {
            return;
        };
        self.name_map.insert(symbol_id, real_name.to_string());
    }

    fn try_extract_from_class_declarator(&mut self, decl: &VariableDeclarator<'_>) {
        let BindingPattern::BindingIdentifier(binding) = &decl.id else {
            return;
        };
        let Some(symbol_id) = binding.symbol_id.get() else {
            return;
        };
        let Some(Expression::ClassExpression(class)) = &decl.init else {
            return;
        };
        let Some(real_name) = Self::real_name_from_class(class, binding.name.as_str()) else {
            return;
        };
        self.name_map.insert(symbol_id, real_name.to_string());
    }
}

impl Default for ClassNameCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ClassNameCollector {
    fn enter_class(&mut self, class: &mut Class<'a>, _ctx: &mut Ctx<'a>) {
        self.try_extract_from_class(class);
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        self.try_extract_from_class_declarator(decl);
    }
}

/// Also collects `.displayName = "X"` patterns:
/// `SomeComponent.displayName = "RealName"`
pub struct DisplayNameCollector {
    name_map: ClassNameMap,
}

impl DisplayNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name_map: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_map(self) -> ClassNameMap {
        self.name_map
    }
}

impl Default for DisplayNameCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DisplayNameCollector {
    fn enter_expression_statement(&mut self, es: &mut oxc_ast::ast::ExpressionStatement<'a>, ctx: &mut Ctx<'a>) {
        // Pattern: X.displayName = "RealName"
        let Expression::AssignmentExpression(assign) = &es.expression else {
            return;
        };
        let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
            return;
        };
        if member.property.name.as_str() != "displayName" {
            return;
        }
        let Expression::Identifier(obj_id) = &member.object else {
            return;
        };
        let Some(symbol_id) = ctx.scoping().get_reference(obj_id.reference_id()).symbol_id() else {
            return;
        };
        let Expression::StringLiteral(s) = &assign.right else {
            return;
        };

        let mangled = obj_id.name.as_str();
        let real_name = s.value.as_str();

        if real_name.len() > 2 && mangled != real_name {
            self.name_map.insert(symbol_id, real_name.to_string());
        }
    }
}

/// Renames class/function identifiers based on collected name maps.
pub struct ClassNameRenamer {
    name_map: ClassNameMap,
    renamed: usize,
}

impl ClassNameRenamer {
    #[must_use]
    pub fn new(name_map: ClassNameMap) -> Self {
        Self { name_map, renamed: 0 }
    }

    #[must_use]
    pub const fn renamed(&self) -> usize {
        self.renamed
    }

    fn lookup(&self, symbol_id: SymbolId) -> Option<&str> {
        self.name_map.get(&symbol_id).map(String::as_str)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ClassNameRenamer {
    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let Some(symbol_id) = ctx.scoping().get_reference(ident.reference_id()).symbol_id() else {
            return;
        };
        if let Some(original) = self.lookup(symbol_id)
            && ident.name.as_str() != original
        {
            ident.name = ctx.ast.ident(original);
            self.renamed += 1;
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
            self.renamed += 1;
        }
    }
}

/// Find `this.name = "..."` in a constructor body.
fn find_this_name_assignment<'a>(body: &FunctionBody<'a>) -> Option<&'a str> {
    for stmt in &body.statements {
        let Statement::ExpressionStatement(es) = stmt else {
            continue;
        };
        let Expression::AssignmentExpression(assign) = &es.expression else {
            continue;
        };
        let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
            continue;
        };
        let Expression::ThisExpression(_) = &member.object else {
            continue;
        };
        if member.property.name.as_str() != "name" {
            continue;
        }
        let Expression::StringLiteral(s) = &assign.right else {
            continue;
        };
        return Some(s.value.as_str());
    }
    None
}

fn is_xml_name(name: &str) -> bool {
    name.starts_with('?') || name.starts_with('#') || name.starts_with('!')
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

    fn collect(code: &str) -> ClassNameMap {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
        let mut prog = parsed.program;
        let mut collector = ClassNameCollector::new();
        let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
        traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
        collector.into_map()
    }

    #[test]
    fn detects_this_name_in_constructor() {
        let code = r#"
            class c5 extends Error {
                constructor(H) {
                    super(H);
                    this.name = "AbortError";
                }
            }
        "#;
        let map = collect(code);
        assert!(map.values().any(|name| name == "AbortError"));
    }

    #[test]
    fn skips_xml_names() {
        let code = r#"
            class x1 {
                constructor() {
                    this.name = "?xml";
                }
            }
        "#;
        let map = collect(code);
        assert!(map.is_empty());
    }

    #[test]
    fn skips_already_readable() {
        let code = r#"
            class AbortError extends Error {
                constructor(H) {
                    super(H);
                    this.name = "AbortError";
                }
            }
        "#;
        let map = collect(code);
        assert!(map.is_empty());
    }
}
