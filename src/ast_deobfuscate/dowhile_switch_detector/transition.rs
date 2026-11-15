use oxc_ast::ast::{
    ArrowFunctionExpression, AssignmentExpression, AssignmentOperator, AssignmentTarget, Class, DoWhileStatement,
    ForInStatement, ForOfStatement, ForStatement, Function, IdentifierReference, Statement, WhileStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;

use super::bindings::Bindings;
use super::value::{StateValue, add, evaluate, subtract};

/// The assignments to the state in one case body that the analysis
/// understands: `state = C` and, run at most once, `state += C` or
/// `state -= C`, where `C` is constant. A write inside a nested function or
/// class is not among them: it may run at any time.
pub(super) struct CaseWrites {
    /// The references of the recognised assignments.
    pub references: Vec<ReferenceId>,
    values: Vec<StateValue>,
    compound: Vec<Compound>,
}

struct Compound {
    operator: AssignmentOperator,
    operand: StateValue,
    in_loop: bool,
}

impl CaseWrites {
    pub fn collect(
        consequent: &[Statement<'_>],
        state: SymbolId,
        bindings: &Bindings<'_>,
        lookup: &impl Fn(&IdentifierReference<'_>) -> Option<StateValue>,
    ) -> Self {
        let mut collector = Collector {
            state,
            bindings,
            lookup,
            loops: 0,
            writes: Self {
                references: Vec::new(),
                values: Vec::new(),
                compound: Vec::new(),
            },
        };
        for statement in consequent {
            collector.visit_statement(statement);
        }
        collector.writes
    }

    /// Every value the case body can leave in the state besides `label`,
    /// the one the case was selected with. `None` when a compound
    /// assignment may start from another value: when the body also assigns
    /// the state, repeats the compound assignment, or is entered by falling
    /// through from the case before.
    pub fn next(self, label: &StateValue, entered_by_fall_through: bool) -> Option<Vec<StateValue>> {
        match self.compound.as_slice() {
            [] => Some(self.values),
            [compound] if self.values.is_empty() && !compound.in_loop && !entered_by_fall_through => {
                let value = match compound.operator {
                    AssignmentOperator::Addition => add(label, &compound.operand)?,
                    _ => subtract(label, &compound.operand)?,
                };
                Some(vec![value])
            }
            _ => None,
        }
    }
}

struct Collector<'b, 's, L> {
    state: SymbolId,
    bindings: &'b Bindings<'s>,
    lookup: &'b L,
    loops: usize,
    writes: CaseWrites,
}

impl<L> Collector<'_, '_, L> {
    fn in_loop(&mut self, walk: impl FnOnce(&mut Self)) {
        self.loops += 1;
        walk(self);
        self.loops -= 1;
    }
}

impl<'a, L: Fn(&IdentifierReference<'_>) -> Option<StateValue>> Visit<'a> for Collector<'_, '_, L> {
    fn visit_function(&mut self, _func: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _arrow: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, _class: &Class<'a>) {}

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.in_loop(|this| walk::walk_for_statement(this, it));
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.in_loop(|this| walk::walk_for_in_statement(this, it));
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.in_loop(|this| walk::walk_for_of_statement(this, it));
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.in_loop(|this| walk::walk_while_statement(this, it));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.in_loop(|this| walk::walk_do_while_statement(this, it));
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(target) = &it.left
            && self.bindings.symbol_of(target) == Some(self.state)
            && let Some(reference) = target.reference_id.get()
            && let Some(value) = evaluate(&it.right, self.lookup)
        {
            let recognised = match it.operator {
                AssignmentOperator::Assign => {
                    self.writes.values.push(value);
                    true
                }
                AssignmentOperator::Addition | AssignmentOperator::Subtraction => {
                    self.writes.compound.push(Compound {
                        operator: it.operator,
                        operand: value,
                        in_loop: self.loops > 0,
                    });
                    true
                }
                _ => false,
            };
            if recognised {
                self.writes.references.push(reference);
            }
        }
        walk::walk_assignment_expression(self, it);
    }
}

/// Whether running `consequent` can go on into the next case's body: no
/// statement at its top level, or in a block there, leaves the case.
pub(super) fn falls_through(consequent: &[Statement<'_>]) -> bool {
    !consequent.iter().any(leaves_case)
}

fn leaves_case(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::BreakStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::ReturnStatement(_)
        | Statement::ThrowStatement(_) => true,
        Statement::BlockStatement(block) => block.body.iter().any(leaves_case),
        _ => false,
    }
}
