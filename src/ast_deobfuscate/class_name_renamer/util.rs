use oxc_ast::ast::{Expression, FunctionBody, Statement};

pub(super) fn find_this_name_assignment<'a>(body: &FunctionBody<'a>) -> Option<&'a str> {
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

pub(super) fn is_xml_name(name: &str) -> bool {
    name.starts_with('?') || name.starts_with('#') || name.starts_with('!')
}
