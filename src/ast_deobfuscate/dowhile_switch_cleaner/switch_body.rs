use oxc_ast::ast::Statement;

pub(super) fn extract_switch_from_body<'a, 'b>(
    body: &'b Statement<'a>,
) -> Option<&'b oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(block) => block.body.iter().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                Some(sw.as_ref())
            } else {
                None
            }
        }),
        _ => None,
    }
}

pub(super) fn extract_switch_from_body_mut<'a, 'b>(
    body: &'b mut Statement<'a>,
) -> Option<&'b mut oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(block) => block.body.iter_mut().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                Some(sw.as_mut())
            } else {
                None
            }
        }),
        _ => None,
    }
}
