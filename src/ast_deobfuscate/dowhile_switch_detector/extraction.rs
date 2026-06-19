use oxc_ast::ast::{BinaryOperator, BindingPattern, Expression, Function, Statement};

use super::transition::classify_transition;
use super::types::{DoWhileCaseInfo, DoWhileDispatcherInfo};

pub(super) fn try_extract_dowhile_dispatcher(
    func: &Function<'_>,
    var_name: Option<String>,
) -> Option<DoWhileDispatcherInfo> {
    let fn_label = var_name
        .as_deref()
        .or_else(|| func.id.as_ref().map(|id| id.name.as_str()))
        .unwrap_or("<anon>");

    if func.r#async || func.generator {
        return None;
    }
    if func.params.items.len() != 2 {
        return None;
    }

    let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
        return None;
    };
    let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
        return None;
    };

    let body = func.body.as_ref()?;
    let state_param = p1.name.as_str();

    let dowhile = body.statements.iter().find_map(|s| {
        if let Statement::DoWhileStatement(dw) = s {
            Some(dw)
        } else {
            None
        }
    })?;

    let Expression::BinaryExpression(test_bin) = &dowhile.test else {
        dwsd_dbg!(
            "[DBG/dwsd] try_extract {}: do-while test is not BinaryExpression",
            fn_label
        );
        return None;
    };
    if !matches!(
        test_bin.operator,
        BinaryOperator::Inequality | BinaryOperator::StrictInequality
    ) {
        dwsd_dbg!(
            "[DBG/dwsd] try_extract {}: do-while test operator is not != or !==",
            fn_label
        );
        return None;
    }
    let Expression::Identifier(test_left) = &test_bin.left else {
        return None;
    };
    if test_left.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(test_right) = &test_bin.right else {
        return None;
    };
    let exit_sentinel = test_right.name.as_str().to_string();

    let switch = extract_switch_from_dowhile_body(&dowhile.body, state_param)?;

    let mut cases = Vec::new();
    let mut has_default = false;

    for case in &switch.cases {
        match &case.test {
            Some(Expression::Identifier(id)) => {
                let label = id.name.as_str().to_string();
                let non_break: Vec<_> = case
                    .consequent
                    .iter()
                    .filter(|s| !matches!(s, Statement::BreakStatement(_)))
                    .collect();
                let body_count = non_break.len();
                let transition = classify_transition(&non_break, state_param, &exit_sentinel);
                cases.push(DoWhileCaseInfo {
                    label,
                    body_statement_count: body_count,
                    transition,
                });
            }
            None => {
                has_default = true;
            }
            _ => {
                dwsd_dbg!("[DBG/dwsd] try_extract {}: case test is not Identifier", fn_label);
            }
        }
    }

    if cases.is_empty() {
        return None;
    }

    let (name, alt_name) = if let Some(vn) = var_name {
        let fn_name = func.id.as_ref().map(|id| id.name.as_str().to_string());
        (vn, fn_name)
    } else {
        let fn_name = func.id.as_ref()?.name.as_str().to_string();
        (fn_name, None)
    };

    dwsd_dbg!(
        "[DBG/dwsd] try_extract {}: SUCCESS with {} cases, exit_sentinel={}",
        name,
        cases.len(),
        exit_sentinel
    );

    Some(DoWhileDispatcherInfo {
        name,
        alt_name,
        state_param: state_param.to_string(),
        args_param: p2.name.as_str().to_string(),
        exit_sentinel,
        cases,
        has_default,
    })
}

fn extract_switch_from_dowhile_body<'a, 'b>(
    body: &'b Statement<'a>,
    state_param: &str,
) -> Option<&'b oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => {
            let Expression::Identifier(disc) = &sw.discriminant else {
                return None;
            };
            if disc.name.as_str() == state_param {
                Some(sw)
            } else {
                None
            }
        }
        Statement::BlockStatement(block) => block.body.iter().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                let Expression::Identifier(disc) = &sw.discriminant else {
                    return None;
                };
                if disc.name.as_str() == state_param {
                    Some(sw.as_ref())
                } else {
                    None
                }
            } else {
                None
            }
        }),
        _ => None,
    }
}
