use oxc_allocator::{ArenaVec, GetAllocator};
use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BindingPattern, Expression, ObjectPropertyKind, PropertyKey, PropertyKind,
    Statement,
};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::types::{CollectedProperty, PropertyValue};
use super::{Ctx, ObjectSparsingConsolidator};

impl ObjectSparsingConsolidator {
    fn is_empty_object(expr: &Expression<'_>) -> bool {
        matches!(expr, Expression::ObjectExpression(obj) if obj.properties.is_empty())
    }

    fn get_empty_object_decl_name(stmt: &Statement<'_>) -> Option<String> {
        let Statement::VariableDeclaration(decl) = stmt else {
            return None;
        };

        if decl.declarations.len() != 1 {
            return None;
        }

        let declarator = &decl.declarations[0];

        let init = declarator.init.as_ref()?;
        if !Self::is_empty_object(init) {
            return None;
        }

        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return None;
        };

        Some(id.name.as_str().to_string())
    }

    fn extract_property_assignment(stmt: &Statement<'_>, target_name: &str) -> Option<(String, PropertyValue)> {
        let Statement::ExpressionStatement(expr_stmt) = stmt else {
            return None;
        };

        let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
            return None;
        };

        if assign.operator != AssignmentOperator::Assign {
            return None;
        }

        let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
            return None;
        };

        let Expression::Identifier(obj_id) = &member.object else {
            return None;
        };

        if obj_id.name.as_str() != target_name {
            return None;
        }

        let prop_name = member.property.name.as_str().to_string();
        let value = Self::extract_value(&assign.right)?;

        Some((prop_name, value))
    }

    fn extract_value(expr: &Expression<'_>) -> Option<PropertyValue> {
        match expr {
            Expression::NumericLiteral(n) => Some(PropertyValue::Number(n.value)),
            Expression::StringLiteral(s) => Some(PropertyValue::String(s.value.as_str().to_string())),
            Expression::BooleanLiteral(b) => Some(PropertyValue::Bool(b.value)),
            Expression::NullLiteral(_) => Some(PropertyValue::Null),
            Expression::Identifier(id) => Some(PropertyValue::Identifier(id.name.as_str().to_string())),
            _ => None,
        }
    }

    fn build_property_value<'a>(value: &PropertyValue, ctx: &mut Ctx<'a>) -> Expression<'a> {
        match value {
            PropertyValue::Number(n) => Expression::new_numeric_literal(SPAN, *n, None, NumberBase::Decimal, ctx),
            PropertyValue::String(s) => {
                let value = ctx.allocator().alloc_str(s);
                Expression::new_string_literal(SPAN, value, None, ctx)
            }
            PropertyValue::Bool(b) => Expression::new_boolean_literal(SPAN, *b, ctx),
            PropertyValue::Null => Expression::new_null_literal(SPAN, ctx),
            PropertyValue::Identifier(name) => {
                let name = ctx.allocator().alloc_str(name);
                Expression::new_identifier(SPAN, name, ctx)
            }
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ObjectSparsingConsolidator {
    fn exit_statements(&mut self, stmts: &mut oxc_allocator::Vec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let mut i = 0;
        while i < stmts.len() {
            let Some(obj_name) = Self::get_empty_object_decl_name(&stmts[i]) else {
                i += 1;
                continue;
            };

            let decl_index = i;
            let mut collected: Vec<CollectedProperty> = Vec::new();
            let mut j = i + 1;

            while j < stmts.len() {
                if let Some((prop_name, value)) = Self::extract_property_assignment(&stmts[j], &obj_name) {
                    collected.push(CollectedProperty {
                        name: prop_name,
                        value,
                        stmt_index: j,
                    });
                    j += 1;
                } else {
                    break;
                }
            }

            if collected.is_empty() {
                i += 1;
                continue;
            }

            deob_trace!(
                "[AST] Consolidating {} properties into object '{}'",
                collected.len(),
                obj_name
            );

            let indices_to_remove: Vec<usize> = collected.iter().map(|p| p.stmt_index).collect();
            for idx in indices_to_remove.into_iter().rev() {
                stmts.remove(idx);
            }

            let mut new_properties: ArenaVec<'a, ObjectPropertyKind<'a>> = ArenaVec::new_in(&ctx.ast);

            for prop in &collected {
                let value_expr = Self::build_property_value(&prop.value, ctx);
                let name = ctx.allocator().alloc_str(&prop.name);
                let key = PropertyKey::new_static_identifier(SPAN, name, ctx);
                let property = ObjectPropertyKind::new_object_property(
                    SPAN,
                    PropertyKind::Init,
                    key,
                    value_expr,
                    false,
                    false,
                    false,
                    ctx,
                );
                new_properties.push(property);
            }

            let new_object = Expression::new_object_expression(SPAN, new_properties, ctx);

            if let Statement::VariableDeclaration(decl) = &mut stmts[decl_index]
                && let Some(declarator) = decl.declarations.first_mut()
            {
                declarator.init = Some(new_object);
            }

            self.rewrites += 1;
        }
    }
}
