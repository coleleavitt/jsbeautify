use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, BindingPattern, Expression, ForStatement, SimpleAssignmentTarget, Statement, SwitchCase, SwitchStatement,
    VariableDeclaration, WhileStatement,
};
use rustc_hash::FxHashMap;

use super::{ControlFlowUnflattener, Ctx};

impl ControlFlowUnflattener {
    pub(super) fn detect_sequence_declaration(&mut self, var_decl: &VariableDeclaration<'_>) {
        for decl in &var_decl.declarations {
            let var_name = match &decl.id {
                BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                _ => continue,
            };

            if let Some(init) = &decl.init {
                if let Some(sequence) = self.extract_split_sequence(init) {
                    deob_trace!("[AST] Found control flow sequence: {var_name} = {sequence:?}");
                    self.detected_sequences.insert(var_name.to_string(), sequence);
                }

                if self.is_zero_initializer(init)
                    && let Some(seq_var) = self.find_associated_sequence_var(var_name)
                {
                    deob_trace!("[AST] Found index variable: {var_name} for sequence {seq_var}");
                    self.detected_index_vars.insert(seq_var, var_name.to_string());
                }
            }
        }
    }

    fn extract_split_sequence(&self, expr: &Expression<'_>) -> Option<Vec<String>> {
        let Expression::CallExpression(call) = expr else {
            return None;
        };

        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };

        if member.property.name.as_str() != "split" {
            return None;
        }

        let string_value = match &member.object {
            Expression::StringLiteral(lit) => lit.value.as_str(),
            _ => return None,
        };

        if call.arguments.len() != 1 {
            return None;
        }

        let separator = match &call.arguments[0] {
            Argument::StringLiteral(lit) => lit.value.as_str(),
            _ => return None,
        };

        if separator != "|" {
            return None;
        }

        let sequence: Vec<String> = string_value.split('|').map(String::from).collect();
        Some(sequence)
    }

    fn is_zero_initializer(&self, expr: &Expression<'_>) -> bool {
        match expr {
            Expression::NumericLiteral(lit) => lit.value == 0.0,
            _ => false,
        }
    }

    fn find_associated_sequence_var(&self, _index_var: &str) -> Option<String> {
        self.detected_sequences.keys().next().cloned()
    }

    pub(super) fn is_control_flow_while(&self, while_stmt: &WhileStatement<'_>) -> bool {
        let is_while_true = match &while_stmt.test {
            Expression::BooleanLiteral(lit) => {
                deob_trace!("[AST]   while test is BooleanLiteral: {}", lit.value);
                lit.value
            }
            other => {
                deob_trace!(
                    "[AST]   while test is not BooleanLiteral: {:?}",
                    std::mem::discriminant(other)
                );
                false
            }
        };

        if !is_while_true {
            deob_trace!("[AST]   Rejecting: not while(true)");
            return false;
        }

        let has_switch = self.body_contains_switch(&while_stmt.body);
        deob_trace!("[AST]   Body contains switch: {has_switch}");
        has_switch
    }

    fn body_contains_switch(&self, stmt: &Statement<'_>) -> bool {
        match stmt {
            Statement::BlockStatement(block) => block.body.iter().any(|s| matches!(s, Statement::SwitchStatement(_))),
            Statement::SwitchStatement(_) => true,
            _ => false,
        }
    }

    pub(super) fn is_control_flow_for(&self, for_stmt: &ForStatement<'_>) -> bool {
        let is_infinite = for_stmt.init.is_none() && for_stmt.test.is_none() && for_stmt.update.is_none();

        if !is_infinite {
            deob_trace!("[AST]   Rejecting: not for(;;)");
            return false;
        }

        deob_trace!("[AST]   for(;;) detected");
        let has_switch = self.body_contains_switch(&for_stmt.body);
        deob_trace!("[AST]   Body contains switch: {has_switch}");
        has_switch
    }

    fn extract_switch_from_for<'a, 'b>(&self, for_stmt: &'b ForStatement<'a>) -> Option<&'b SwitchStatement<'a>> {
        let Statement::BlockStatement(block) = &for_stmt.body else {
            return None;
        };

        for stmt in &block.body {
            if let Statement::SwitchStatement(switch) = stmt {
                return Some(switch);
            }
        }
        None
    }

    fn extract_switch_from_while<'a, 'b>(&self, while_stmt: &'b WhileStatement<'a>) -> Option<&'b SwitchStatement<'a>> {
        let Statement::BlockStatement(block) = &while_stmt.body else {
            return None;
        };

        for stmt in &block.body {
            if let Statement::SwitchStatement(switch) = stmt {
                return Some(switch);
            }
        }
        None
    }

    fn is_sequence_access(&self, expr: &Expression<'_>) -> Option<String> {
        deob_trace!("[AST]   Checking discriminant: {:?}", std::mem::discriminant(expr));

        let array_name = match expr {
            Expression::ComputedMemberExpression(member) => {
                deob_trace!("[AST]   Discriminant is ComputedMemberExpression");
                if let Expression::Identifier(ident) = &member.object {
                    deob_trace!("[AST]   Array name: {}", ident.name);
                    ident.name.as_str()
                } else {
                    deob_trace!("[AST]   Member object is not Identifier");
                    return None;
                }
            }
            Expression::UpdateExpression(update) => {
                deob_trace!("[AST]   Discriminant is UpdateExpression");
                if let SimpleAssignmentTarget::ComputedMemberExpression(member) = &update.argument {
                    if let Expression::Identifier(ident) = &member.object {
                        deob_trace!("[AST]   Array name: {}", ident.name);
                        ident.name.as_str()
                    } else {
                        deob_trace!("[AST]   Member object is not Identifier");
                        return None;
                    }
                } else {
                    deob_trace!("[AST]   Update argument is not ComputedMemberExpression");
                    return None;
                }
            }
            _ => {
                deob_trace!("[AST]   Discriminant is neither ComputedMemberExpression nor UpdateExpression");
                return None;
            }
        };

        if self.detected_sequences.contains_key(array_name) {
            deob_trace!("[AST]   Found sequence access for: {array_name}");
            return Some(array_name.to_string());
        }

        deob_trace!(
            "[AST]   Array {} not in detected sequences: {:?}",
            array_name,
            self.detected_sequences.keys().collect::<Vec<_>>()
        );
        None
    }

    pub(super) fn extract_unflattened_while<'a>(
        &self,
        while_stmt: &WhileStatement<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<OxcVec<'a, Statement<'a>>> {
        let switch = self.extract_switch_from_while(while_stmt)?;
        let seq_var = self.is_sequence_access(&switch.discriminant)?;
        let sequence = self.detected_sequences.get(&seq_var)?;
        self.extract_from_switch_cases(switch, sequence, ctx)
    }

    pub(super) fn extract_unflattened_for<'a>(
        &self,
        for_stmt: &ForStatement<'a>,
        ctx: &mut Ctx<'a>,
    ) -> Option<OxcVec<'a, Statement<'a>>> {
        let switch = self.extract_switch_from_for(for_stmt)?;
        let seq_var = self.is_sequence_access(&switch.discriminant)?;
        let sequence = self.detected_sequences.get(&seq_var)?;
        self.extract_from_switch_cases(switch, sequence, ctx)
    }

    fn extract_from_switch_cases<'a>(
        &self,
        switch: &SwitchStatement<'a>,
        sequence: &[String],
        ctx: &mut Ctx<'a>,
    ) -> Option<OxcVec<'a, Statement<'a>>> {
        let mut case_map: FxHashMap<String, &SwitchCase<'a>> = FxHashMap::default();
        for case in &switch.cases {
            if let Some(test) = &case.test {
                let case_value = match test {
                    Expression::StringLiteral(lit) => lit.value.as_str().to_string(),
                    Expression::NumericLiteral(lit) => lit.raw.map_or_else(|| lit.value.to_string(), |r| r.to_string()),
                    _ => continue,
                };
                case_map.insert(case_value, case);
            }
        }

        let mut result = OxcVec::new_in(ctx);
        for step in sequence {
            if let Some(case) = case_map.get(step) {
                for stmt in &case.consequent {
                    if self.should_keep_statement(stmt) {
                        result.push(stmt.clone_in_with_semantic_ids(ctx.allocator()));
                    }
                }
            }
        }

        if result.is_empty() {
            return None;
        }
        Some(result)
    }

    const fn should_keep_statement(&self, stmt: &Statement<'_>) -> bool {
        !matches!(stmt, Statement::ContinueStatement(_) | Statement::BreakStatement(_))
    }
}
