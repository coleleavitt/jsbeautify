use oxc_ast::ast::{Function, Statement};

/// The cases to keep in one dispatcher's `switch`.
pub(super) struct Prune {
    pub loop_index: usize,
    pub switch_index: Option<usize>,
    /// Per case, in source order, whether some call can reach it.
    pub reached: Vec<bool>,
}

impl Prune {
    /// Removes the unreached cases from `func`'s `switch`. Returns how many
    /// it removed.
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
        let mut reached = self.reached.iter();
        switch.cases.retain(|_| reached.next().copied().unwrap_or(true));
        self.reached.iter().filter(|&&reached| !reached).count()
    }
}
