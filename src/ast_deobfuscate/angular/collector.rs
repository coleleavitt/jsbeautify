use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, Expression, PropertyDefinition, Statement,
    VariableDeclarator,
};
use oxc_span::{GetSpan, Span};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::DefKind;
use super::naming::{action_name, class_name_for, object_property};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// A variable that holds a class whose own binding is `inner`:
/// `var outer = class inner {}` or `var outer = (() => { class inner {} return inner })()`.
#[derive(Clone, Copy)]
pub(super) struct Wrapper {
    pub(super) outer: SymbolId,
    /// Span of the initializer; references to `outer` inside it would be
    /// captured if `inner` took the same name.
    pub(super) init_span: Span,
}

/// Read-only traversal collecting Angular naming evidence.
#[derive(Default)]
pub struct AngularNameCollector {
    /// Class symbol named by a definition's `type` → derived class name.
    pub(super) classes: FxHashMap<SymbolId, String>,
    /// Inner class binding → the variable it is assigned to.
    pub(super) wrappers: FxHashMap<SymbolId, Wrapper>,
    /// `createAction("[Source] Event", ...)` result binding → action name.
    pub(super) actions: FxHashMap<SymbolId, String>,
}

impl AngularNameCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn record_definition(&mut self, key: &str, value: &Expression<'_>, ctx: &Ctx<'_>) {
        let Some(kind) = DefKind::from_key(key) else { return };
        let Expression::CallExpression(call) = value.without_parentheses() else {
            return;
        };
        let Some(Expression::ObjectExpression(def)) = call.arguments.first().and_then(|a| a.as_expression()) else {
            return;
        };
        let Some(Expression::Identifier(ty)) = object_property(def, "type") else {
            return;
        };
        let Some(symbol) = ctx.scoping().get_reference(ty.reference_id()).symbol_id() else {
            return;
        };
        if let Some(name) = class_name_for(kind, def) {
            self.classes.insert(symbol, name);
        }
    }
}

/// `inner` for `class inner {}` returned from a no-argument IIFE body.
fn iife_returned_class(call: &CallExpression<'_>) -> Option<SymbolId> {
    if !call.arguments.is_empty() {
        return None;
    }
    let statements = match call.callee.without_parentheses() {
        Expression::ArrowFunctionExpression(arrow) => &arrow.get_function_body()?.statements,
        Expression::FunctionExpression(func) => &func.body.as_ref()?.statements,
        _ => return None,
    };
    let Some(Statement::ReturnStatement(ret)) = statements.last() else {
        return None;
    };
    let Some(Expression::Identifier(returned)) = ret.argument.as_ref() else {
        return None;
    };
    statements.iter().find_map(|stmt| match stmt {
        Statement::ClassDeclaration(class) => class
            .id
            .as_ref()
            .filter(|id| id.name == returned.name)
            .and_then(|id| id.symbol_id.get()),
        _ => None,
    })
}

impl<'a> Traverse<'a, DeobfuscateState> for AngularNameCollector {
    fn enter_property_definition(&mut self, prop: &mut PropertyDefinition<'a>, ctx: &mut Ctx<'a>) {
        if !prop.r#static {
            return;
        }
        if let (Some(key), Some(value)) = (prop.key.static_name(), prop.value.as_ref()) {
            self.record_definition(&key, value, ctx);
        }
    }

    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, ctx: &mut Ctx<'a>) {
        let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
            return;
        };
        self.record_definition(member.property.name.as_str(), &assign.right, ctx);
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(binding) = &decl.id else {
            return;
        };
        let (Some(outer), Some(init)) = (binding.symbol_id.get(), decl.init.as_ref()) else {
            return;
        };
        let inner = match init.without_parentheses() {
            Expression::ClassExpression(class) => class.id.as_ref().and_then(|id| id.symbol_id.get()),
            Expression::CallExpression(call) => {
                if let [first, ..] = call.arguments.as_slice()
                    && call.arguments.len() <= 2
                    && let Some(Expression::StringLiteral(action_type)) = first.as_expression()
                    && let Some(name) = action_name(&action_type.value)
                {
                    self.actions.insert(outer, name);
                }
                iife_returned_class(call)
            }
            _ => None,
        };
        if let Some(inner) = inner {
            self.wrappers.insert(
                inner,
                Wrapper {
                    outer,
                    init_span: init.span(),
                },
            );
        }
    }
}
