use oxc_ast::ast::{AssignmentOperator, AssignmentTarget, Expression, Statement};

use super::types::{CompoundOp, StateTransition};

pub(super) fn classify_transition(stmts: &[&Statement<'_>], state_param: &str, exit_sentinel: &str) -> StateTransition {
    if stmts.is_empty() {
        return StateTransition::Unknown;
    }

    let last = stmts[stmts.len() - 1];
    if has_return(last) {
        return StateTransition::Return;
    }

    if let Some(transition) = check_if_conditional(last, state_param) {
        return transition;
    }

    for stmt in stmts.iter().rev() {
        if let Some(next_state) = extract_state_assignment(stmt, state_param) {
            if next_state == exit_sentinel {
                return StateTransition::LoopExit;
            }
            return StateTransition::Sequential(next_state);
        }
        if let Some(compound) = extract_compound_assignment(stmt, state_param) {
            return compound;
        }
    }

    StateTransition::Unknown
}

fn has_return(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(block) => block.body.iter().any(has_return),
        _ => false,
    }
}

fn extract_state_assignment(stmt: &Statement<'_>, state_param: &str) -> Option<String> {
    let Statement::ExpressionStatement(es) = stmt else {
        if let Statement::BlockStatement(block) = stmt {
            for s in block.body.iter().rev() {
                if let Some(name) = extract_state_assignment(s, state_param) {
                    return Some(name);
                }
            }
        }
        return None;
    };
    let Expression::AssignmentExpression(assign) = &es.expression else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign {
        return None;
    }
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    if target.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(rhs) = &assign.right else {
        return None;
    };
    Some(rhs.name.as_str().to_string())
}

fn extract_compound_assignment(stmt: &Statement<'_>, state_param: &str) -> Option<StateTransition> {
    let Statement::ExpressionStatement(es) = stmt else {
        if let Statement::BlockStatement(block) = stmt {
            for s in block.body.iter().rev() {
                if let Some(t) = extract_compound_assignment(s, state_param) {
                    return Some(t);
                }
            }
        }
        return None;
    };
    let Expression::AssignmentExpression(assign) = &es.expression else {
        return None;
    };
    let op = match assign.operator {
        AssignmentOperator::Addition => CompoundOp::AddAssign,
        AssignmentOperator::Subtraction => CompoundOp::SubAssign,
        _ => return None,
    };
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    if target.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(rhs) = &assign.right else {
        return None;
    };
    Some(StateTransition::Compound {
        op,
        rhs_name: rhs.name.as_str().to_string(),
    })
}

fn check_if_conditional(stmt: &Statement<'_>, state_param: &str) -> Option<StateTransition> {
    let stmt = if let Statement::BlockStatement(block) = stmt {
        block.body.last()?
    } else {
        stmt
    };

    let Statement::IfStatement(ifs) = stmt else {
        return None;
    };

    let consequent_state = extract_state_from_branch(&ifs.consequent, state_param);
    let alternate_state = ifs
        .alternate
        .as_ref()
        .and_then(|alt| extract_state_from_branch(alt, state_param));

    match (consequent_state, alternate_state) {
        (Some(a), Some(b)) => Some(StateTransition::Conditional(a, b)),
        _ => None,
    }
}

fn extract_state_from_branch(stmt: &Statement<'_>, state_param: &str) -> Option<String> {
    match stmt {
        Statement::ExpressionStatement(_) => extract_state_assignment(stmt, state_param),
        Statement::BlockStatement(block) => {
            for s in block.body.iter().rev() {
                if let Some(name) = extract_state_assignment(s, state_param) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}
