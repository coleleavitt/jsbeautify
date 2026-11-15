use oxc_allocator::{ArenaVec, GetAllocator};
use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BindingPattern, Expression, IdentifierReference, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, Statement, VariableDeclarationKind,
};
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;
use oxc_syntax::symbol::{SymbolFlags, SymbolId};
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::{is_global_reference, redirectable_symbols};

use super::types::{CollectedProperty, PropertyValue};
use super::{Ctx, ObjectSparsingConsolidator};

impl ObjectSparsingConsolidator {
    fn is_empty_object(expr: &Expression<'_>) -> bool {
        matches!(expr, Expression::ObjectExpression(obj) if obj.properties.is_empty())
    }

    /// The binding of `var|let|const NAME = {};`.
    fn empty_object_declaration(&self, stmt: &Statement<'_>) -> Option<SymbolId> {
        let Statement::VariableDeclaration(decl) = stmt else {
            return None;
        };
        if !matches!(
            decl.kind,
            VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
        ) {
            return None;
        }
        let [declarator] = decl.declarations.as_slice() else {
            return None;
        };
        if !declarator.init.as_ref().is_some_and(Self::is_empty_object) {
            return None;
        }
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return None;
        };
        id.symbol_id.get().filter(|symbol| !self.redirectable.contains(symbol))
    }

    /// `NAME.key = value;` where `NAME` resolves to `target` and evaluating
    /// `value` can neither run code nor throw.
    fn extract_property_assignment(
        &self,
        stmt: &Statement<'_>,
        target: SymbolId,
        ctx: &Ctx<'_>,
    ) -> Option<(String, PropertyValue)> {
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
        // In a literal, `__proto__: v` sets the prototype rather than a
        // property, and a second one is a SyntaxError.
        if member.optional || member.property.name == "__proto__" || resolve(obj_id, ctx) != Some(target) {
            return None;
        }
        let prop_name = member.property.name.as_str().to_string();
        let value = self.extract_value(&assign.right, target, ctx)?;
        Some((prop_name, value))
    }

    fn extract_value(&self, expr: &Expression<'_>, target: SymbolId, ctx: &Ctx<'_>) -> Option<PropertyValue> {
        match expr {
            Expression::NumericLiteral(n) => Some(PropertyValue::Number(n.value)),
            Expression::StringLiteral(s) => s.value.as_str().map(|value| PropertyValue::String(value.to_string())),
            Expression::BooleanLiteral(b) => Some(PropertyValue::Bool(b.value)),
            Expression::NullLiteral(_) => Some(PropertyValue::Null),
            Expression::Identifier(id) => self
                .reads_safely(id, target, ctx)
                .then(|| PropertyValue::Identifier(id.name.as_str().to_string())),
            _ => None,
        }
    }

    /// Reading `id` cannot throw, run code or see the object half-built: a
    /// `var`, parameter or function other than `target` that no other code
    /// can redefine, or the global `undefined`, `NaN` or `Infinity`.
    fn reads_safely(&self, id: &IdentifierReference<'_>, target: SymbolId, ctx: &Ctx<'_>) -> bool {
        resolve(id, ctx).map_or_else(
            || matches!(id.name.as_str(), "undefined" | "NaN" | "Infinity") && is_global_reference(ctx, &id.name),
            |symbol| {
                let scoping = ctx.scoping();
                symbol != target
                    && scoping
                        .symbol_flags(symbol)
                        .intersects(SymbolFlags::FunctionScopedVariable | SymbolFlags::Function)
                    && !self.redirectable.contains(&symbol)
                    && !is_global_binding(self.source_type, scoping, symbol)
            },
        )
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

fn resolve(id: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> Option<SymbolId> {
    // Earlier Phase 11 passes add identifiers without a reference.
    ctx.scoping().get_reference(id.reference_id.get()?).symbol_id()
}

impl<'a> Traverse<'a, DeobfuscateState> for ObjectSparsingConsolidator {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.redirectable = redirectable_symbols(program, ctx.scoping());
        self.source_type = program.source_type;
    }

    fn exit_statements(&mut self, stmts: &mut oxc_allocator::Vec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        let mut i = 0;
        while i < stmts.len() {
            let Some(target) = self.empty_object_declaration(&stmts[i]) else {
                i += 1;
                continue;
            };

            let decl_index = i;
            let mut collected: Vec<CollectedProperty> = Vec::new();
            let mut j = i + 1;

            while j < stmts.len() {
                if let Some((prop_name, value)) = self.extract_property_assignment(&stmts[j], target, ctx) {
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
                ctx.scoping().symbol_name(target)
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
