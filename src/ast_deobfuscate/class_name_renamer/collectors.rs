use oxc_ast::ast::{BindingPattern, Class, Expression, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::bun::export_renamer::names::{is_valid_identifier, sanitize_identifier};
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::util::{constructor_this_name, is_xml_name};
use super::{ClassNameMap, Ctx};

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
    pub const fn map(&self) -> &ClassNameMap {
        &self.name_map
    }

    fn real_name_from_class<'a>(class: &Class<'a>, mangled: &str) -> Option<&'a str> {
        // Skip if already a long readable name
        if mangled.len() > 5 && mangled.chars().any(|c| c.is_ascii_lowercase()) {
            return None;
        }

        constructor_this_name(class)
            .filter(|real_name| *real_name != mangled && real_name.len() > 2 && !is_xml_name(real_name))
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

        if real_name.len() <= 2 {
            return;
        }

        // React compound components declare dotted display names
        // (`Accordion.Content`, `Menu.Item`). The renamer writes this string
        // straight into binding positions, so a dot would emit
        // `var Accordion.Content = ...` -- a member expression where a binding
        // identifier must be, which fails the re-parse that guards this pass
        // and aborts deobfuscation for the whole file. Sanitize to
        // `Accordion_Content` so the readable name survives as a legal
        // identifier instead of being dropped.
        let candidate = if is_valid_identifier(real_name) {
            real_name.to_string()
        } else {
            sanitize_identifier(real_name)
        };

        if candidate != mangled {
            self.name_map.insert(symbol_id, candidate);
        }
    }
}
