use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BindingPattern, BooleanLiteral, Expression, IdentifierName,
    IdentifierReference, NullLiteral, NumericLiteral, ObjectExpression, ObjectProperty, ObjectPropertyKind,
    PropertyKey, PropertyKind, Statement, StringLiteral,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use std::cell::Cell;

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
            PropertyValue::Number(n) => Expression::NumericLiteral(ctx.ast.alloc(NumericLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *n,
                raw: None,
                base: oxc_syntax::number::NumberBase::Decimal,
            })),
            PropertyValue::String(s) => Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: ctx.ast.str(s),
                raw: None,
                lone_surrogates: false,
            })),
            PropertyValue::Bool(b) => Expression::BooleanLiteral(ctx.ast.alloc(BooleanLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                value: *b,
            })),
            PropertyValue::Null => Expression::NullLiteral(ctx.ast.alloc(NullLiteral {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
            })),
            PropertyValue::Identifier(name) => Expression::Identifier(ctx.ast.alloc(IdentifierReference {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(name),
                reference_id: Cell::default(),
            })),
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

            eprintln!(
                "[AST] Consolidating {} properties into object '{}'",
                collected.len(),
                obj_name
            );

            let indices_to_remove: Vec<usize> = collected.iter().map(|p| p.stmt_index).collect();
            for idx in indices_to_remove.into_iter().rev() {
                stmts.remove(idx);
            }

            let mut new_properties: oxc_allocator::Vec<'a, ObjectPropertyKind<'a>> =
                oxc_allocator::Vec::new_in(ctx.ast.allocator);

            for prop in &collected {
                let value_expr = Self::build_property_value(&prop.value, ctx);
                let property = ObjectPropertyKind::ObjectProperty(ctx.ast.alloc(ObjectProperty {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    kind: PropertyKind::Init,
                    key: PropertyKey::StaticIdentifier(ctx.ast.alloc(IdentifierName {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        name: ctx.ast.ident(&prop.name),
                    })),
                    value: value_expr,
                    method: false,
                    shorthand: false,
                    computed: false,
                }));
                new_properties.push(property);
            }

            let new_object = Expression::ObjectExpression(ctx.ast.alloc(ObjectExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                properties: new_properties,
            }));

            if let Statement::VariableDeclaration(decl) = &mut stmts[decl_index]
                && let Some(declarator) = decl.declarations.first_mut()
            {
                declarator.init = Some(new_object);
            }

            self.changed = true;
        }
    }
}
