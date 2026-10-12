use oxc_ast::ast::{Argument, AssignmentTarget, BindingPattern, CallExpression, Expression, Function, Statement};

use crate::ast_deobfuscate::state::OffsetOperation;

use super::array_extraction::{extract_offset_assignment, extract_offset_from_binary, extract_string_array};
use super::checksum::extract_checksum_terms;
use super::decoder_ref::find_decoder_ref_in_body;
use super::pass::SelfDefendingStringArray;
use super::types::{DecoderCandidate, RotationCandidate};

impl SelfDefendingStringArray {
    pub(super) fn detect_wrapper(&mut self, func: &Function<'_>, func_name: &str) {
        let Some(body) = &func.body else { return };
        if !func.params.items.is_empty() {
            return;
        }

        let stmts = &body.statements;
        if stmts.len() < 2 {
            return;
        }

        let strings = match &stmts[0] {
            Statement::VariableDeclaration(var_decl) if var_decl.declarations.len() == 1 => {
                let decl = &var_decl.declarations[0];
                let Some(init) = &decl.init else { return };
                let Expression::ArrayExpression(arr) = init else {
                    return;
                };
                extract_string_array(&arr.elements)
            }
            _ => return,
        };

        let Some(strings) = strings else { return };
        if strings.is_empty() {
            return;
        }

        let has_self_replace = stmts.iter().any(|s| {
            if let Statement::ExpressionStatement(expr_stmt) = s
                && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
                && let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left
            {
                return id.name.as_str() == func_name;
            }
            false
        });

        if !has_self_replace {
            return;
        }

        deob_trace!(
            "[SELF-DEFENDING] Found array wrapper: {func_name} with {} strings",
            strings.len()
        );
        self.wrappers.insert(func_name.to_string(), strings);
    }

    /// Detect decoder without requiring wrapper to be known yet.
    /// Collects any function with shape: `idx = idx - OFFSET; const arr = SOME_FN(); return arr[idx];`
    pub(super) fn detect_decoder_candidate(&mut self, func: &Function<'_>, func_name: &str) {
        let Some(body) = &func.body else { return };

        if func.params.items.is_empty() || func.params.items.len() > 2 {
            return;
        }

        let param_name = match &func.params.items[0].pattern {
            BindingPattern::BindingIdentifier(id) => id.name.as_str(),
            _ => return,
        };

        let stmts = &body.statements;
        let mut offset = 0i32;
        let mut offset_op = OffsetOperation::None;
        let mut wrapper_ref: Option<String> = None;
        let mut found_array_access = false;

        for stmt in stmts {
            match stmt {
                Statement::ExpressionStatement(expr_stmt) => {
                    if let Some((off, op)) = extract_offset_assignment(&expr_stmt.expression, param_name) {
                        offset = off;
                        offset_op = op;
                    }
                }
                Statement::VariableDeclaration(var_decl) => {
                    for decl in &var_decl.declarations {
                        if let Some(init) = &decl.init
                            && let Expression::CallExpression(call) = init
                            && call.arguments.is_empty()
                            && let Expression::Identifier(id) = &call.callee
                        {
                            wrapper_ref = Some(id.name.as_str().to_string());
                        }
                    }
                }
                Statement::ReturnStatement(ret) => {
                    if let Some(arg) = &ret.argument
                        && let Expression::ComputedMemberExpression(member) = arg
                    {
                        let index_uses_param = match &member.expression {
                            Expression::Identifier(id) => id.name.as_str() == param_name,
                            _ => false,
                        };
                        if index_uses_param {
                            found_array_access = true;
                        }
                    }
                }
                _ => {}
            }
        }

        // Also handle inline offset in return: return WRAPPER()[idx - OFFSET]
        if !found_array_access {
            for stmt in stmts {
                if let Statement::ReturnStatement(ret) = stmt
                    && let Some(arg) = &ret.argument
                    && let Expression::ComputedMemberExpression(member) = arg
                {
                    if let Expression::CallExpression(call) = &member.object
                        && call.arguments.is_empty()
                        && let Expression::Identifier(id) = &call.callee
                    {
                        wrapper_ref = Some(id.name.as_str().to_string());
                    }
                    if let Expression::BinaryExpression(bin) = &member.expression {
                        if let Some((off, op)) = extract_offset_from_binary(bin, param_name)
                            && offset_op == OffsetOperation::None
                        {
                            offset = off;
                            offset_op = op;
                        }
                        found_array_access = true;
                    } else if let Expression::Identifier(id) = &member.expression
                        && id.name.as_str() == param_name
                    {
                        found_array_access = true;
                    }
                }
            }
        }

        let Some(wrapper) = wrapper_ref else { return };
        if !found_array_access && offset_op == OffsetOperation::None {
            return;
        }

        deob_trace!(
            "[SELF-DEFENDING] Found decoder candidate: {func_name} -> wrapper_ref={wrapper}, offset={offset_op:?} {offset}"
        );
        self.decoder_candidates.insert(
            func_name.to_string(),
            DecoderCandidate {
                wrapper_ref: wrapper,
                offset,
                offset_op,
            },
        );
    }

    /// Detect rotation IIFE without requiring wrapper/decoder to be known.
    /// Collects: `(function(a, b) { ... while(true) { try { const c = EXPR; ... } } })(SOME_NAME, NUMBER)`
    pub(super) fn detect_rotation_candidate(&mut self, call: &CallExpression<'_>) {
        let func = match &call.callee {
            Expression::FunctionExpression(f) => f.as_ref(),
            Expression::ParenthesizedExpression(p) => {
                if let Expression::FunctionExpression(f) = &p.expression {
                    f.as_ref()
                } else {
                    return;
                }
            }
            _ => return,
        };

        if func.params.items.len() != 2 {
            return;
        }

        let wrapper_ref = match call.arguments.first() {
            Some(Argument::Identifier(id)) => id.name.as_str().to_string(),
            _ => return,
        };

        let target = match call.arguments.get(1) {
            Some(Argument::NumericLiteral(lit)) => lit.value,
            _ => return,
        };

        let Some(body) = &func.body else { return };

        // Extract checksum terms
        let terms = extract_checksum_terms(&body.statements);
        if terms.is_empty() {
            return;
        }

        // Find decoder reference: look for `const d = SOME_NAME;` or calls to SOME_NAME(0xNNN)
        let decoder_ref = find_decoder_ref_in_body(&body.statements);
        let Some(decoder_ref) = decoder_ref else {
            return;
        };

        deob_trace!(
            "[SELF-DEFENDING] Found rotation candidate: wrapper_ref={wrapper_ref}, decoder_ref={decoder_ref}, target={target}, terms={}",
            terms.len()
        );

        self.rotation_candidates.push(RotationCandidate {
            wrapper_ref,
            decoder_ref,
            terms,
            target,
        });
    }

    /// Detect top-level `const X = Y;` aliases (e.g. `const _0x126786 = _0x5eba;`)
    pub(super) fn detect_alias(&mut self, stmt: &Statement<'_>) {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init
                    && let Expression::Identifier(target_id) = init
                    && let BindingPattern::BindingIdentifier(bind_id) = &decl.id
                {
                    let alias = bind_id.name.as_str().to_string();
                    let target = target_id.name.as_str().to_string();
                    self.top_level_aliases.insert(alias, target);
                }
            }
        }
    }
}
