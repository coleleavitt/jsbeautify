use oxc_ast::ast::{AssignmentTarget, Expression, SimpleAssignmentTarget};

pub(super) use super::types::MemberKey;

pub(super) fn member_key_from_target(target: &AssignmentTarget<'_>) -> Option<MemberKey> {
    let SimpleAssignmentTarget::StaticMemberExpression(sme) = target.as_simple_assignment_target()? else {
        return None;
    };
    member_key_from_static_member(&sme.object, sme.property.name.as_str())
}

pub(super) fn member_key_from_static_member(object: &Expression<'_>, prop: &str) -> Option<MemberKey> {
    match object {
        Expression::ThisExpression(_) => Some(MemberKey::This(prop.to_string())),
        Expression::Identifier(id) => Some(MemberKey::Bound(id.name.to_string(), prop.to_string())),
        _ => None,
    }
}
