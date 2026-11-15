use oxc_ast::ast::{
    ArrowFunctionExpression, Class, Function, Statement, SwitchCase, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_syntax::scope::ScopeFlags;

/// The cases to keep in one dispatcher's `switch`.
pub(super) struct Prune {
    pub loop_index: usize,
    pub switch_index: Option<usize>,
    /// Per case, in source order, whether some call can reach it.
    pub reached: Vec<bool>,
}

impl Prune {
    /// Removes the unreached cases from `func`'s `switch`, except those
    /// that declare a binding. Returns how many it removed.
    pub fn apply(&self, func: &mut Function<'_>) -> usize {
        let Some(body) = &mut func.body else { return 0 };
        let Some(Statement::DoWhileStatement(dowhile)) = body.statements.get_mut(self.loop_index) else {
            return 0;
        };
        let switch = match (&mut dowhile.body, self.switch_index) {
            (Statement::SwitchStatement(switch), None) => switch,
            (Statement::BlockStatement(block), Some(index)) => match block.body.get_mut(index) {
                Some(Statement::SwitchStatement(switch)) => switch,
                _ => return 0,
            },
            _ => return 0,
        };
        if switch.cases.len() != self.reached.len() {
            return 0;
        }
        let before = switch.cases.len();
        let mut reached = self.reached.iter();
        switch
            .cases
            .retain(|case| reached.next().copied().unwrap_or(true) || declares_binding(case));
        before - switch.cases.len()
    }
}

/// Whether `case` declares a binding that code outside it can see: a
/// `let`, `const`, `class` or function at its top level belongs to the
/// whole `switch` body, a `var` anywhere in it to the dispatcher function,
/// and so does a sloppy-mode function declared in one of its blocks
/// (Annex B). Another case can read or write such a binding, so the case
/// stays even when it never runs.
fn declares_binding(case: &SwitchCase<'_>) -> bool {
    let mut finder = Declarations(false);
    for statement in &case.consequent {
        if matches!(
            statement,
            Statement::VariableDeclaration(_) | Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_)
        ) {
            return true;
        }
        finder.visit_statement(statement);
    }
    finder.0
}

/// Finds `var` and function declarations, not looking inside nested
/// functions and classes, whose own scopes hold what they declare.
struct Declarations(bool);

impl<'a> Visit<'a> for Declarations {
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.0 |= it.kind == VariableDeclarationKind::Var;
    }

    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        self.0 |= it.is_declaration();
    }

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, _it: &Class<'a>) {}
}
