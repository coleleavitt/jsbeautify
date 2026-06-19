use oxc_ast::ast::{BindingPattern, Expression, Function, Statement};
use rustc_hash::FxHashMap;

use super::types::{CaseInfo, DispatcherInfo};

pub(super) fn try_extract_dispatcher(func: &Function<'_>, var_name: Option<String>) -> Option<DispatcherInfo> {
    let fn_label = var_name
        .as_deref()
        .or_else(|| func.id.as_ref().map(|id| id.name.as_str()))
        .unwrap_or("<anon>");

    if func.r#async || func.generator {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: async={} generator={}",
            fn_label,
            func.r#async,
            func.generator
        );
        return None;
    }
    if func.params.items.len() != 2 {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params.len()={} (need 2)",
            fn_label,
            func.params.items.len()
        );
        return None;
    }

    let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params[0] not BindingIdentifier",
            fn_label
        );
        return None;
    };
    let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params[1] not BindingIdentifier",
            fn_label
        );
        return None;
    };

    let body = func.body.as_ref()?;

    // Allow var decls / expr stmts before the switch (e.g., `var xF = rQ; switch(...)`)
    let switch = body.statements.iter().find_map(|s| {
        if let Statement::SwitchStatement(sw) = s {
            Some(sw)
        } else {
            None
        }
    });
    let Some(switch) = switch else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: no SwitchStatement in {} body stmts",
            fn_label,
            body.statements.len()
        );
        return None;
    };
    // Reject bodies with control flow beyond var/expr/switch (prevents false positives)
    let non_switch_non_var = body
        .statements
        .iter()
        .filter(|s| {
            !matches!(
                s,
                Statement::SwitchStatement(_) | Statement::VariableDeclaration(_) | Statement::ExpressionStatement(_)
            )
        })
        .count();
    if non_switch_non_var > 0 {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: {} non-switch/non-var/non-expr stmts in body",
            fn_label,
            non_switch_non_var
        );
        return None;
    }

    let Expression::Identifier(disc) = &switch.discriminant else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: discriminant is not Identifier",
            fn_label
        );
        return None;
    };
    if disc.name.as_str() != p1.name.as_str() {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: discriminant '{}' != p1 '{}'",
            fn_label,
            disc.name,
            p1.name
        );
        return None;
    }

    let mut cases = FxHashMap::default();
    let mut has_default = false;

    for case in &switch.cases {
        match &case.test {
            Some(Expression::Identifier(id)) => {
                let label = id.name.as_str().to_string();
                let body_count = case
                    .consequent
                    .iter()
                    .filter(|s| !matches!(s, Statement::BreakStatement(_)))
                    .count();
                cases.insert(
                    label.clone(),
                    CaseInfo {
                        state_name: label,
                        body_statement_count: body_count,
                    },
                );
            }
            None => {
                has_default = true;
            }
            _ => {
                cff_dbg!(
                    "[DBG/disp] try_extract {}: case test is not Identifier (skipping case)",
                    fn_label
                );
            }
        }
    }

    if cases.is_empty() {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: no valid cases found (switch has {} cases)",
            fn_label,
            switch.cases.len()
        );
        return None;
    }

    let (name, alt_name) = if let Some(vn) = var_name {
        let fn_name = func.id.as_ref().map(|id| id.name.as_str().to_string());
        (vn, fn_name)
    } else {
        let fn_name = func.id.as_ref()?.name.as_str().to_string();
        (fn_name, None)
    };

    cff_dbg!("[DBG/disp] try_extract {}: SUCCESS with {} cases", name, cases.len());

    Some(DispatcherInfo {
        name,
        alt_name,
        state_param: p1.name.as_str().to_string(),
        args_param: p2.name.as_str().to_string(),
        cases,
        has_default,
    })
}
