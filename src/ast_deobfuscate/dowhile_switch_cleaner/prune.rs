use oxc_ast::ast::{Expression, Function, Statement};

use super::switch_body::{extract_switch_from_body, extract_switch_from_body_mut};
use super::{Ctx, DoWhileSwitchCleaner};

impl DoWhileSwitchCleaner {
    pub(super) fn try_prune_dispatcher<'a>(&mut self, name: &str, func: &Function<'a>, _ctx: &mut Ctx<'a>) {
        let Some(reachable) = self.reachable_per_dispatcher.get(name) else {
            return;
        };
        if !self.dispatchers.contains_key(name) {
            return;
        }
        let Some(body) = &func.body else { return };

        let switch = body.statements.iter().find_map(|s| {
            if let Statement::DoWhileStatement(dw) = s {
                extract_switch_from_body(&dw.body)
            } else {
                None
            }
        });
        let Some(switch) = switch else { return };

        let total = switch.cases.len();
        let dead_count = switch
            .cases
            .iter()
            .filter(|c| {
                c.test
                    .as_ref()
                    .and_then(|t| {
                        if let Expression::Identifier(id) = t {
                            Some(id.name.as_str())
                        } else {
                            None
                        }
                    })
                    .is_some_and(|label| !reachable.contains(label))
            })
            .count();

        if dead_count > 0 {
            eprintln!(
                "[DOWHILE] dispatcher {name}: {dead_count}/{total} cases are dead (not pruning immutable FunctionDeclaration)"
            );
        }
    }

    pub(super) fn try_prune_dispatcher_mut<'a>(&mut self, name: &str, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        let Some(reachable) = self.reachable_per_dispatcher.get(name) else {
            return;
        };
        if !self.dispatchers.contains_key(name) {
            return;
        }
        let Some(body) = &mut func.body else { return };

        let switch = body.statements.iter_mut().find_map(|s| {
            if let Statement::DoWhileStatement(dw) = s {
                extract_switch_from_body_mut(&mut dw.body)
            } else {
                None
            }
        });
        let Some(switch) = switch else { return };

        let total = switch.cases.len();
        let before = switch.cases.len();

        switch.cases.retain(|case| {
            let Some(test) = &case.test else {
                return true; // keep default case
            };
            let Expression::Identifier(id) = test else {
                return true; // keep non-identifier cases
            };
            reachable.contains(id.name.as_str())
        });

        let removed = before - switch.cases.len();
        if removed > 0 {
            eprintln!("[DOWHILE] pruned {removed}/{total} dead cases from {name}");
            self.pruned_cases += removed;
        }
    }
}
