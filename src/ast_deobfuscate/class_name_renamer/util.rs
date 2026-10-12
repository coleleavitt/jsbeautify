use oxc_ast::ast::{Class, ClassElement, Expression, FunctionBody, MethodDefinitionKind, Statement};

/// The string a class constructor assigns to `this.name`.
pub fn constructor_this_name<'a>(class: &Class<'a>) -> Option<&'a str> {
    class.body.body.iter().find_map(|element| match element {
        ClassElement::MethodDefinition(method) if method.kind == MethodDefinitionKind::Constructor => method
            .value
            .body
            .as_ref()
            .and_then(|body| find_this_name_assignment(body)),
        _ => None,
    })
}

fn find_this_name_assignment<'a>(body: &FunctionBody<'a>) -> Option<&'a str> {
    body.statements.iter().find_map(|stmt| {
        let Statement::ExpressionStatement(es) = stmt else {
            return None;
        };
        match &es.expression {
            // Minifiers join constructor statements: `super(x), this.name = "E"`.
            Expression::SequenceExpression(seq) => seq.expressions.iter().find_map(this_name_assignment),
            expr => this_name_assignment(expr),
        }
    })
}

/// `"E"` of `this.name = "E"`.
fn this_name_assignment<'a>(expr: &Expression<'a>) -> Option<&'a str> {
    let Expression::AssignmentExpression(assign) = expr else {
        return None;
    };
    let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
        return None;
    };
    if !matches!(member.object, Expression::ThisExpression(_)) || member.property.name != "name" {
        return None;
    }
    match &assign.right {
        Expression::StringLiteral(s) => Some(s.value.as_str()),
        _ => None,
    }
}

pub(super) fn is_xml_name(name: &str) -> bool {
    name.starts_with('?') || name.starts_with('#') || name.starts_with('!')
}
