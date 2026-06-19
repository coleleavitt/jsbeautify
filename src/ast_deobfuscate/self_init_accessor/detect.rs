use oxc_ast::ast::{BindingPattern, Expression, Function, Statement};

use super::types::InitKind;

pub(super) fn classify_init(expr: &Expression<'_>) -> Option<InitKind> {
    match expr {
        Expression::ArrayExpression(a) if a.elements.is_empty() => Some(InitKind::EmptyArray),
        Expression::ObjectExpression(o) if o.properties.is_empty() => Some(InitKind::EmptyObject),
        Expression::NewExpression(ne) => match &ne.callee {
            Expression::Identifier(id) if id.name.as_str() == "Object" && ne.arguments.is_empty() => {
                Some(InitKind::NewObject)
            }
            Expression::Identifier(id) if id.name.as_str() == "Array" && ne.arguments.is_empty() => {
                Some(InitKind::NewArray)
            }
            _ => None,
        },
        Expression::CallExpression(call) => {
            if let Expression::StaticMemberExpression(sme) = &call.callee {
                if let Expression::Identifier(obj) = &sme.object
                    && obj.name.as_str() == "Object"
                    && sme.property.name.as_str() == "create"
                    && call.arguments.len() == 1
                {
                    if matches!(call.arguments[0].as_expression(), Some(Expression::NullLiteral(_))) {
                        return Some(InitKind::ObjectCreateNull);
                    }
                    return Some(InitKind::ObjectCreateProto);
                }
                if let Expression::ArrayExpression(arr) = &sme.object
                    && arr.elements.is_empty()
                    && matches!(sme.property.name.as_str(), "entries" | "keys" | "values")
                    && call.arguments.is_empty()
                {
                    let name: &'static str = match sme.property.name.as_str() {
                        "entries" => "entries",
                        "keys" => "keys",
                        "values" => "values",
                        _ => unreachable!(),
                    };
                    return Some(InitKind::ArrayIter(name));
                }
            }
            None
        }
        Expression::ParenthesizedExpression(p) => classify_init(&p.expression),
        _ => None,
    }
}

pub(super) fn is_self_init_accessor_named(func: &Function<'_>) -> Option<(String, InitKind)> {
    let name = func.id.as_ref()?.name.as_str().to_string();
    if !func.params.items.is_empty() {
        return None;
    }
    let body = func.body.as_ref()?;
    if body.statements.len() != 3 {
        return None;
    }

    let Statement::VariableDeclaration(decl) = &body.statements[0] else {
        return None;
    };
    if decl.declarations.len() != 1 {
        return None;
    }
    let local_name = match &decl.declarations[0].id {
        BindingPattern::BindingIdentifier(id) => id.name.as_str(),
        _ => return None,
    };
    let init_kind = classify_init(decl.declarations[0].init.as_ref()?)?;

    let Statement::ExpressionStatement(expr_stmt) = &body.statements[1] else {
        return None;
    };
    let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
        return None;
    };
    let target_ok = matches!(
        &assign.left,
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) if id.name.as_str() == name
    );
    if !target_ok {
        return None;
    }
    let Expression::FunctionExpression(reassigned) = &assign.right else {
        return None;
    };
    let reassigned_body = reassigned.body.as_ref()?;
    if reassigned_body.statements.len() != 1 {
        return None;
    }
    let Statement::ReturnStatement(ret) = &reassigned_body.statements[0] else {
        return None;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return None;
    };
    if ret_id.name.as_str() != local_name {
        return None;
    }

    let Statement::ReturnStatement(ret) = &body.statements[2] else {
        return None;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return None;
    };
    if ret_id.name.as_str() != local_name {
        return None;
    }

    Some((name, init_kind))
}
