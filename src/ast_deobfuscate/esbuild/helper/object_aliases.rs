use oxc_ast::ast::{BindingPattern, Expression, VariableDeclarator};

use super::{EsbuildHelperCollector, EsbuildHelperInfo, EsbuildHelperKind, ObjectAliasKind};

impl EsbuildHelperCollector {
    pub(super) fn try_detect_object_alias(&mut self, declarator: &VariableDeclarator<'_>) {
        let Some(init) = &declarator.init else {
            return;
        };

        let var_name = match &declarator.id {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str().to_string(),
            _ => return,
        };

        let Expression::StaticMemberExpression(member) = init else {
            return;
        };

        let Expression::Identifier(obj) = &member.object else {
            return;
        };

        if obj.name.as_str() != "Object" {
            return;
        }

        let kind = match member.property.name.as_str() {
            "defineProperty" => ObjectAliasKind::DefineProperty,
            "getOwnPropertyNames" => ObjectAliasKind::GetOwnPropertyNames,
            "getOwnPropertyDescriptor" => ObjectAliasKind::GetOwnPropertyDescriptor,
            "getPrototypeOf" => ObjectAliasKind::GetPrototypeOf,
            "create" => ObjectAliasKind::Create,
            _ => return,
        };

        eprintln!("[ESBUILD] Detected Object alias: {var_name} = Object.{kind:?}");
        self.object_aliases.insert(var_name.clone(), kind);
        self.helpers.insert(
            var_name.clone(),
            EsbuildHelperInfo::new(var_name, EsbuildHelperKind::ObjectAlias(kind), declarator.span.start),
        );
    }

    pub(super) fn try_detect_prototype_alias(&mut self, declarator: &VariableDeclarator<'_>) {
        let Some(init) = &declarator.init else {
            return;
        };

        let var_name = match &declarator.id {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str().to_string(),
            _ => return,
        };

        let Expression::StaticMemberExpression(member) = init else {
            return;
        };

        let Expression::StaticMemberExpression(inner) = &member.object else {
            return;
        };

        let Expression::Identifier(obj) = &inner.object else {
            return;
        };

        if obj.name.as_str() != "Object" || inner.property.name.as_str() != "prototype" {
            return;
        }

        let kind = match member.property.name.as_str() {
            "hasOwnProperty" => ObjectAliasKind::HasOwnProperty,
            "propertyIsEnumerable" => ObjectAliasKind::PropertyIsEnumerable,
            _ => return,
        };

        eprintln!("[ESBUILD] Detected Object.prototype alias: {var_name} = Object.prototype.{kind:?}");
        self.object_aliases.insert(var_name.clone(), kind);
        self.helpers.insert(
            var_name.clone(),
            EsbuildHelperInfo::new(var_name, EsbuildHelperKind::ObjectAlias(kind), declarator.span.start),
        );
    }
}
