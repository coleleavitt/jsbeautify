use oxc_ast::ast::{
    BinaryOperator, BindingPattern, Expression, Function, FunctionBody, IdentifierReference, Statement, SwitchStatement,
};
use oxc_ast_visit::Visit;
use rustc_hash::FxHashSet;

use super::bindings::{Bindings, Constants, Position};
use super::transition::{CaseWrites, falls_through};
use super::types::{DoWhileCase, DoWhileDispatcher};
use super::value::evaluate;

/// The dispatcher `func` is, when it has the shape
/// `function (state, args) { … do { switch (state) { case C: … } } while (state != SENTINEL); … }`
/// and its state only ever holds known constants: every case label is a
/// constant, and every write to the state is an assignment of a constant
/// in a case body. Constants are those declared before `declared`, the
/// declarator that assigns the function, in the same statement list: they
/// are initialised whenever the function can run.
pub(super) fn extract(
    func: &Function<'_>,
    bindings: &Bindings<'_>,
    constants: &Constants,
    declared: Position,
) -> Option<DoWhileDispatcher> {
    if func.r#async || func.generator || func.params.rest.is_some() {
        return None;
    }
    let [state_param, args_param] = func.params.items.as_slice() else {
        return None;
    };
    let (BindingPattern::BindingIdentifier(state), BindingPattern::BindingIdentifier(_)) =
        (&state_param.pattern, &args_param.pattern)
    else {
        return None;
    };
    if state_param.initializer.is_some() || args_param.initializer.is_some() {
        return None;
    }
    let state = state.symbol_id.get()?;
    let body = func.body.as_ref()?;
    // A sloppy function's `arguments[0]` is the state under another name.
    if !bindings.is_tracked(state) || uses_arguments(body) {
        return None;
    }

    let reads_state =
        |expr: &Expression<'_>| matches!(expr, Expression::Identifier(id) if bindings.symbol_of(id) == Some(state));
    let (loop_index, dowhile) = body
        .statements
        .iter()
        .enumerate()
        .find_map(|(i, statement)| match statement {
            Statement::DoWhileStatement(dowhile) => Some((i, dowhile)),
            _ => None,
        })?;
    let Expression::BinaryExpression(test) = &dowhile.test else {
        return None;
    };
    if !matches!(
        test.operator,
        BinaryOperator::Inequality | BinaryOperator::StrictInequality
    ) || !reads_state(&test.left)
        || !matches!(test.right, Expression::Identifier(_))
    {
        return None;
    }
    let (switch_index, switch): (_, &SwitchStatement<'_>) = match &dowhile.body {
        Statement::SwitchStatement(switch) => (None, switch),
        Statement::BlockStatement(block) => {
            block
                .body
                .iter()
                .enumerate()
                .find_map(|(i, statement)| match statement {
                    Statement::SwitchStatement(switch) if reads_state(&switch.discriminant) => {
                        Some((Some(i), &**switch))
                    }
                    _ => None,
                })?
        }
        _ => return None,
    };
    if !reads_state(&switch.discriminant) || switch.cases.is_empty() {
        return None;
    }

    let lookup = |id: &IdentifierReference<'_>| constants.lookup(bindings, id, declared);
    let mut recognised = FxHashSet::default();
    let mut cases = Vec::with_capacity(switch.cases.len());
    let mut entered_by_fall_through = false;
    for case in &switch.cases {
        // A `default` case runs for values no label matches.
        let label = evaluate(case.test.as_ref()?, &lookup)?;
        let writes = CaseWrites::collect(&case.consequent, state, bindings, &lookup);
        recognised.extend(writes.references.iter().copied());
        let next = writes.next(&label, entered_by_fall_through)?;
        let falls_through = falls_through(&case.consequent);
        cases.push(DoWhileCase {
            label,
            next,
            falls_through,
        });
        entered_by_fall_through = falls_through;
    }
    let scoping = bindings.scoping;
    let only_recognised_writes = scoping
        .get_resolved_reference_ids(state)
        .iter()
        .all(|reference| !scoping.get_reference(*reference).is_write() || recognised.contains(reference));
    only_recognised_writes.then_some(DoWhileDispatcher {
        loop_index,
        switch_index,
        cases,
        entries: Vec::new(),
    })
}

fn uses_arguments(body: &FunctionBody<'_>) -> bool {
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            self.0 |= it.name == "arguments";
        }
    }
    let mut finder = Finder(false);
    finder.visit_function_body(body);
    finder.0
}
