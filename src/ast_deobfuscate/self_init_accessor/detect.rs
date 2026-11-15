use oxc_ast::ast::{
    Argument, AssignmentOperator, AssignmentTarget, BindingPattern, CallExpression, Expression, Function,
    IdentifierReference, Statement, VariableDeclarationKind,
};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;

use super::types::InitKind;

/// Name resolution for the detector: the program's scoping and the scopes
/// inside `with` bodies, where a free name may read a property instead.
pub(super) struct Resolver<'s> {
    pub(super) scoping: &'s Scoping,
    pub(super) with_scopes: &'s FxHashSet<ScopeId>,
}

impl Resolver<'_> {
    fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        self.scoping.get_reference(id.reference_id.get()?).symbol_id()
    }

    /// `id` names the global builtin `name`: no binding, no `with` object.
    fn is_global(&self, id: &IdentifierReference<'_>, name: &str) -> bool {
        id.name == name
            && id.reference_id.get().is_some_and(|reference| {
                let reference = self.scoping.get_reference(reference);
                reference.symbol_id().is_none() && !self.with_scopes.contains(&reference.scope_id())
            })
    }

    pub(super) fn classify_init(&self, expr: &Expression<'_>) -> Option<InitKind> {
        match expr {
            Expression::ArrayExpression(a) if a.elements.is_empty() => Some(InitKind::EmptyArray),
            Expression::ObjectExpression(o) if o.properties.is_empty() => Some(InitKind::EmptyObject),
            Expression::NewExpression(ne) if ne.arguments.is_empty() => match &ne.callee {
                Expression::Identifier(id) if self.is_global(id, "Object") => Some(InitKind::NewObject),
                Expression::Identifier(id) if self.is_global(id, "Array") => Some(InitKind::NewArray),
                _ => None,
            },
            Expression::CallExpression(call) if !call.optional => self.classify_call(call),
            Expression::ParenthesizedExpression(p) => self.classify_init(&p.expression),
            _ => None,
        }
    }

    /// `Object.create(null)`, `Object.create({})` and `[].entries()`,
    /// `[].keys()` or `[].values()`.
    fn classify_call(&self, call: &CallExpression<'_>) -> Option<InitKind> {
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return None;
        };
        match (&sme.object, sme.property.name.as_str(), call.arguments.as_slice()) {
            (Expression::Identifier(obj), "create", [argument]) if self.is_global(obj, "Object") => match argument {
                Argument::NullLiteral(_) => Some(InitKind::ObjectCreateNull),
                Argument::ObjectExpression(proto) if proto.properties.is_empty() => Some(InitKind::ObjectCreateProto),
                _ => None,
            },
            (Expression::ArrayExpression(arr), method, []) if arr.elements.is_empty() => match method {
                "entries" => Some(InitKind::ArrayIter("entries")),
                "keys" => Some(InitKind::ArrayIter("keys")),
                "values" => Some(InitKind::ArrayIter("values")),
                _ => None,
            },
            _ => None,
        }
    }

    /// The initialiser of `function F() { var x = <init>; F = function () {
    /// return x; }; return x; }`, with `F` and `x` matched by `SymbolId`.
    pub(super) fn self_init_accessor(&self, func: &Function<'_>) -> Option<InitKind> {
        let accessor = func.id.as_ref()?.symbol_id.get()?;
        if func.r#async || func.generator || !func.params.items.is_empty() || func.params.rest.is_some() {
            return None;
        }
        let [
            Statement::VariableDeclaration(decl),
            Statement::ExpressionStatement(reassign),
            Statement::ReturnStatement(ret),
        ] = func.body.as_ref()?.statements.as_slice()
        else {
            return None;
        };
        let [declarator] = decl.declarations.as_slice() else {
            return None;
        };
        let BindingPattern::BindingIdentifier(local) = &declarator.id else {
            return None;
        };
        let local = local.symbol_id.get()?;
        if !matches!(
            decl.kind,
            VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
        ) {
            return None;
        }
        let init = self.classify_init(declarator.init.as_ref()?)?;

        let Expression::AssignmentExpression(assign) = &reassign.expression else {
            return None;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return None;
        };
        let Expression::FunctionExpression(getter) = &assign.right else {
            return None;
        };
        if assign.operator != AssignmentOperator::Assign
            || self.symbol(target) != Some(accessor)
            || getter.r#async
            || getter.generator
        {
            return None;
        }
        let [Statement::ReturnStatement(getter_ret)] = getter.body.as_ref()?.statements.as_slice() else {
            return None;
        };
        let returns_local = |argument: Option<&Expression<'_>>| matches!(argument, Some(Expression::Identifier(id)) if self.symbol(id) == Some(local));
        (returns_local(getter_ret.argument.as_ref()) && returns_local(ret.argument.as_ref())).then_some(init)
    }
}
