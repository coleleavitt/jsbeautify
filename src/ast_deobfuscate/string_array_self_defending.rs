//! Self-defending string array pattern (javascript-obfuscator modern default)
//!
//! Handles the three-piece pattern produced by javascript-obfuscator with
//! `selfDefending: true` + `rotateStringArray: true`:
//!
//! 1. **Self-replacing array wrapper**: `function NAME() { const arr = [...]; NAME = function(){return arr}; return NAME(); }`
//! 2. **Decoder function**: `function NAME(idx, _key) { idx = idx - OFFSET; const arr = WRAPPER(); return arr[idx]; }`
//! 3. **Checksum rotation IIFE**: brute-forces `arr.push(arr.shift())` until a
//!    `parseInt`-based checksum matches a target literal.
//!
//! All three pieces can appear in any order in the source. Detection collects
//! candidates without cross-referencing; `finalize()` resolves relationships.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentTarget, BinaryExpression, BinaryOperator, BindingPattern,
    CallExpression, Expression, Function, Statement, UnaryOperator,
};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::{DecoderInfo, DecoderType, DeobfuscateState, OffsetOperation, StringArrayInfo};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

#[derive(Debug, Clone)]
struct ChecksumTerm {
    indices: Vec<i32>,
    divisors: Vec<f64>,
    sign: i8,
}

/// Candidate decoder: references a wrapper by name but we don't validate the wrapper exists yet.
#[derive(Debug, Clone)]
struct DecoderCandidate {
    wrapper_ref: String,
    offset: i32,
    offset_op: OffsetOperation,
}

/// Candidate rotation IIFE: references wrapper + decoder by name.
#[derive(Debug, Clone)]
struct RotationCandidate {
    wrapper_ref: String,
    decoder_ref: String,
    terms: Vec<ChecksumTerm>,
    target: f64,
}

pub struct SelfDefendingStringArray {
    wrappers: FxHashMap<String, Vec<String>>,
    decoder_candidates: FxHashMap<String, DecoderCandidate>,
    rotation_candidates: Vec<RotationCandidate>,
    /// All identifier names used as `const X = NAME;` aliases at top-level scope
    top_level_aliases: FxHashMap<String, String>,
}

impl SelfDefendingStringArray {
    #[must_use]
    pub fn new() -> Self {
        Self {
            wrappers: FxHashMap::default(),
            decoder_candidates: FxHashMap::default(),
            rotation_candidates: Vec::new(),
            top_level_aliases: FxHashMap::default(),
        }
    }

    pub fn finalize(&mut self, state: &mut DeobfuscateState) {
        // Resolve decoder candidates: keep only those whose wrapper_ref is a known wrapper
        let valid_decoders: FxHashMap<String, DecoderCandidate> = self
            .decoder_candidates
            .iter()
            .filter(|(_, c)| self.wrappers.contains_key(&c.wrapper_ref))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Resolve rotation candidates
        for rot in &self.rotation_candidates {
            if !self.wrappers.contains_key(&rot.wrapper_ref) {
                continue;
            }
            let decoder = match valid_decoders.get(&rot.decoder_ref) {
                Some(d) => d,
                None => continue,
            };

            let Some(strings) = self.wrappers.get_mut(&rot.wrapper_ref) else {
                continue;
            };

            let rotation_count =
                brute_force_rotation(strings, &rot.terms, rot.target, decoder.offset, decoder.offset_op);

            match rotation_count {
                Some(count) => {
                    apply_rotation(strings, count);
                    eprintln!(
                        "[SELF-DEFENDING] Applied checksum rotation to {}: {} rotations (target={})",
                        rot.wrapper_ref, count, rot.target
                    );
                }
                None => {
                    eprintln!(
                        "[SELF-DEFENDING] WARNING: Could not solve checksum for {} (target={})",
                        rot.wrapper_ref, rot.target
                    );
                }
            }
        }

        for (name, strings) in &self.wrappers {
            let rotated = self.rotation_candidates.iter().any(|r| r.wrapper_ref == *name);
            state.string_arrays.insert(
                name.clone(),
                StringArrayInfo {
                    var_name: name.clone(),
                    strings: strings.clone(),
                    rotated,
                    rotation_count: 0,
                },
            );
        }

        for (dec_name, candidate) in &valid_decoders {
            state.decoders.insert(
                dec_name.clone(),
                DecoderInfo {
                    function_name: dec_name.clone(),
                    array_name: candidate.wrapper_ref.clone(),
                    offset: candidate.offset,
                    offset_operation: candidate.offset_op,
                    decoder_type: DecoderType::Simple,
                },
            );
        }

        // Also register aliases as decoders (e.g. `const _0x126786 = _0x5eba;`)
        for (alias, target) in &self.top_level_aliases {
            if let Some(dec) = valid_decoders.get(target) {
                if !state.decoders.contains_key(alias) {
                    state.decoders.insert(
                        alias.clone(),
                        DecoderInfo {
                            function_name: alias.clone(),
                            array_name: dec.wrapper_ref.clone(),
                            offset: dec.offset,
                            offset_operation: dec.offset_op,
                            decoder_type: DecoderType::Simple,
                        },
                    );
                }
            }
        }
    }

    fn detect_wrapper(&mut self, func: &Function<'_>, func_name: &str) {
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
            if let Statement::ExpressionStatement(expr_stmt) = s {
                if let Expression::AssignmentExpression(assign) = &expr_stmt.expression {
                    if let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left {
                        return id.name.as_str() == func_name;
                    }
                }
            }
            false
        });

        if !has_self_replace {
            return;
        }

        eprintln!(
            "[SELF-DEFENDING] Found array wrapper: {func_name} with {} strings",
            strings.len()
        );
        self.wrappers.insert(func_name.to_string(), strings);
    }

    /// Detect decoder without requiring wrapper to be known yet.
    /// Collects any function with shape: `idx = idx - OFFSET; const arr = SOME_FN(); return arr[idx];`
    fn detect_decoder_candidate(&mut self, func: &Function<'_>, func_name: &str) {
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
                        if let Some(init) = &decl.init {
                            if let Expression::CallExpression(call) = init {
                                if call.arguments.is_empty() {
                                    if let Expression::Identifier(id) = &call.callee {
                                        wrapper_ref = Some(id.name.as_str().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                Statement::ReturnStatement(ret) => {
                    if let Some(arg) = &ret.argument {
                        if let Expression::ComputedMemberExpression(member) = arg {
                            let index_uses_param = match &member.expression {
                                Expression::Identifier(id) => id.name.as_str() == param_name,
                                _ => false,
                            };
                            if index_uses_param {
                                found_array_access = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Also handle inline offset in return: return WRAPPER()[idx - OFFSET]
        if !found_array_access {
            for stmt in stmts {
                if let Statement::ReturnStatement(ret) = stmt {
                    if let Some(arg) = &ret.argument {
                        if let Expression::ComputedMemberExpression(member) = arg {
                            if let Expression::CallExpression(call) = &member.object {
                                if call.arguments.is_empty() {
                                    if let Expression::Identifier(id) = &call.callee {
                                        wrapper_ref = Some(id.name.as_str().to_string());
                                    }
                                }
                            }
                            if let Expression::BinaryExpression(bin) = &member.expression {
                                if let Some((off, op)) = extract_offset_from_binary(bin, param_name) {
                                    if offset_op == OffsetOperation::None {
                                        offset = off;
                                        offset_op = op;
                                    }
                                }
                                found_array_access = true;
                            } else if let Expression::Identifier(id) = &member.expression {
                                if id.name.as_str() == param_name {
                                    found_array_access = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        let Some(wrapper) = wrapper_ref else { return };
        if !found_array_access && offset_op == OffsetOperation::None {
            return;
        }

        eprintln!(
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
    fn detect_rotation_candidate(&mut self, call: &CallExpression<'_>) {
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

        eprintln!(
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
    fn detect_alias(&mut self, stmt: &Statement<'_>) {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init {
                    if let Expression::Identifier(target_id) = init {
                        if let BindingPattern::BindingIdentifier(bind_id) = &decl.id {
                            let alias = bind_id.name.as_str().to_string();
                            let target = target_id.name.as_str().to_string();
                            self.top_level_aliases.insert(alias, target);
                        }
                    }
                }
            }
        }
    }
}

impl Default for SelfDefendingStringArray {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfDefendingStringArray {
    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        if let Statement::FunctionDeclaration(func_decl) = stmt {
            let func_name = func_decl.id.as_ref().map_or("", |id| id.name.as_str());
            if !func_name.is_empty() {
                self.detect_wrapper(func_decl, func_name);
                self.detect_decoder_candidate(func_decl, func_name);
            }
        }

        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            match &expr_stmt.expression {
                Expression::CallExpression(call) => {
                    self.detect_rotation_candidate(call);
                }
                Expression::ParenthesizedExpression(paren) => {
                    if let Expression::CallExpression(call) = &paren.expression {
                        self.detect_rotation_candidate(call);
                    }
                }
                _ => {}
            }
        }

        self.detect_alias(stmt);
    }
}

fn extract_string_array(elements: &[ArrayExpressionElement<'_>]) -> Option<Vec<String>> {
    let mut strings = Vec::with_capacity(elements.len());
    for elem in elements {
        match elem {
            ArrayExpressionElement::StringLiteral(lit) => {
                strings.push(lit.value.to_string());
            }
            _ => return None,
        }
    }
    Some(strings)
}

fn extract_offset_assignment(expr: &Expression<'_>, param_name: &str) -> Option<(i32, OffsetOperation)> {
    if let Expression::AssignmentExpression(assign) = expr {
        if let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left {
            if target.name.as_str() == param_name {
                if let Expression::BinaryExpression(bin) = &assign.right {
                    return extract_offset_from_binary(bin, param_name);
                }
            }
        }
    }
    None
}

fn extract_offset_from_binary(bin: &BinaryExpression<'_>, param_name: &str) -> Option<(i32, OffsetOperation)> {
    let left_is_param = matches!(&bin.left, Expression::Identifier(id) if id.name.as_str() == param_name);
    if !left_is_param {
        return None;
    }

    let offset = if let Expression::NumericLiteral(lit) = &bin.right {
        lit.value as i32
    } else {
        return None;
    };

    let op = match bin.operator {
        BinaryOperator::Subtraction => OffsetOperation::Subtract,
        BinaryOperator::Addition => OffsetOperation::Add,
        _ => return None,
    };

    Some((offset, op))
}

/// Find the decoder function name referenced inside the IIFE body.
/// Does NOT require knowing which names are decoders — just finds `const d = SOME_NAME;`
/// or the function name used in `parseInt(NAME(...))` calls.
fn find_decoder_ref_in_body(stmts: &[Statement<'_>]) -> Option<String> {
    // Look for `const d = SOME_NAME;` pattern
    for stmt in stmts {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init {
                    if let Expression::Identifier(id) = init {
                        let name = id.name.as_str();
                        if name.starts_with("_0x") || name.starts_with("_0X") {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // Scan for parseInt(NAME(...)) calls to find the decoder name
    for stmt in stmts {
        if let Some(name) = scan_stmt_for_parseint_decoder(stmt) {
            return Some(name);
        }
    }

    None
}

fn scan_stmt_for_parseint_decoder(stmt: &Statement<'_>) -> Option<String> {
    match stmt {
        Statement::WhileStatement(w) => scan_stmt_for_parseint_decoder(&w.body),
        Statement::BlockStatement(b) => {
            for s in &b.body {
                if let Some(name) = scan_stmt_for_parseint_decoder(s) {
                    return Some(name);
                }
            }
            None
        }
        Statement::TryStatement(t) => {
            for s in &t.block.body {
                if let Some(name) = scan_stmt_for_parseint_decoder(s) {
                    return Some(name);
                }
            }
            None
        }
        Statement::VariableDeclaration(v) => {
            for decl in &v.declarations {
                if let Some(init) = &decl.init {
                    if let Some(name) = scan_expr_for_parseint_decoder(init) {
                        return Some(name);
                    }
                }
            }
            None
        }
        Statement::ExpressionStatement(e) => scan_expr_for_parseint_decoder(&e.expression),
        _ => None,
    }
}

fn scan_expr_for_parseint_decoder(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::CallExpression(call) => {
            // Check if this is parseInt(DECODER_NAME(...))
            if let Expression::Identifier(id) = &call.callee {
                if id.name.as_str() == "parseInt" {
                    if let Some(first_arg) = call.arguments.first() {
                        if let Some(arg_expr) = first_arg.as_expression() {
                            if let Expression::CallExpression(inner) = arg_expr {
                                if let Expression::Identifier(inner_id) = &inner.callee {
                                    return Some(inner_id.name.as_str().to_string());
                                }
                            }
                        }
                    }
                }
            }
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    if let Some(name) = scan_expr_for_parseint_decoder(e) {
                        return Some(name);
                    }
                }
            }
            None
        }
        Expression::BinaryExpression(bin) => {
            scan_expr_for_parseint_decoder(&bin.left).or_else(|| scan_expr_for_parseint_decoder(&bin.right))
        }
        Expression::UnaryExpression(un) => scan_expr_for_parseint_decoder(&un.argument),
        Expression::ParenthesizedExpression(p) => scan_expr_for_parseint_decoder(&p.expression),
        _ => None,
    }
}

fn extract_checksum_terms(stmts: &[Statement<'_>]) -> Vec<ChecksumTerm> {
    for stmt in stmts {
        match stmt {
            Statement::WhileStatement(w) => {
                let result = extract_checksum_terms_from_stmt(&w.body);
                if !result.is_empty() {
                    return result;
                }
            }
            Statement::BlockStatement(b) => {
                let result = extract_checksum_terms(&b.body);
                if !result.is_empty() {
                    return result;
                }
            }
            _ => {
                let result = extract_checksum_terms_from_stmt(stmt);
                if !result.is_empty() {
                    return result;
                }
            }
        }
    }
    Vec::new()
}

fn extract_checksum_terms_from_stmt(stmt: &Statement<'_>) -> Vec<ChecksumTerm> {
    match stmt {
        Statement::TryStatement(t) => {
            for s in &t.block.body {
                let result = extract_checksum_terms_from_stmt(s);
                if !result.is_empty() {
                    return result;
                }
            }
            Vec::new()
        }
        Statement::VariableDeclaration(v) => {
            for decl in &v.declarations {
                if let Some(init) = &decl.init {
                    let mut terms = Vec::new();
                    collect_additive_terms(init, 1, &mut terms);
                    if !terms.is_empty() {
                        return terms;
                    }
                }
            }
            Vec::new()
        }
        Statement::BlockStatement(b) => extract_checksum_terms(&b.body),
        Statement::IfStatement(_) => Vec::new(),
        _ => Vec::new(),
    }
}

fn collect_additive_terms(expr: &Expression<'_>, sign: i8, terms: &mut Vec<ChecksumTerm>) {
    match expr {
        Expression::BinaryExpression(bin) => match bin.operator {
            BinaryOperator::Addition => {
                collect_additive_terms(&bin.left, sign, terms);
                collect_additive_terms(&bin.right, sign, terms);
            }
            BinaryOperator::Subtraction => {
                collect_additive_terms(&bin.left, sign, terms);
                collect_additive_terms(&bin.right, -sign, terms);
            }
            BinaryOperator::Division => {
                let (inner_sign, inner_expr) = strip_unary_neg(&bin.left);
                if let Some(idx) = extract_parseint_decoder_idx(inner_expr) {
                    if let Expression::NumericLiteral(lit) = &bin.right {
                        terms.push(ChecksumTerm {
                            indices: vec![idx],
                            divisors: vec![lit.value],
                            sign: sign * inner_sign,
                        });
                    }
                }
            }
            BinaryOperator::Multiplication => {
                let left_term = extract_single_term(&bin.left, 1);
                let right_term = extract_single_term(&bin.right, 1);
                if let (Some(lt), Some(rt)) = (left_term, right_term) {
                    let mut indices = lt.indices;
                    indices.extend(rt.indices);
                    let mut divisors = lt.divisors;
                    divisors.extend(rt.divisors);
                    let combined_sign = sign * lt.sign * rt.sign;
                    terms.push(ChecksumTerm {
                        indices,
                        divisors,
                        sign: combined_sign,
                    });
                }
            }
            _ => {}
        },
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            collect_additive_terms(&un.argument, -sign, terms);
        }
        Expression::ParenthesizedExpression(p) => {
            collect_additive_terms(&p.expression, sign, terms);
        }
        _ => {
            if let Some(term) = extract_single_term(expr, sign) {
                terms.push(term);
            }
        }
    }
}

fn extract_single_term(expr: &Expression<'_>, sign: i8) -> Option<ChecksumTerm> {
    match expr {
        Expression::BinaryExpression(bin) if bin.operator == BinaryOperator::Division => {
            // Handle: parseInt(d(IDX)) / DIVISOR  or  -parseInt(d(IDX)) / DIVISOR
            let (inner_sign, inner_expr) = strip_unary_neg(&bin.left);
            let idx = extract_parseint_decoder_idx(inner_expr)?;
            if let Expression::NumericLiteral(lit) = &bin.right {
                Some(ChecksumTerm {
                    indices: vec![idx],
                    divisors: vec![lit.value],
                    sign: sign * inner_sign,
                })
            } else {
                None
            }
        }
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            extract_single_term(&un.argument, -sign)
        }
        Expression::ParenthesizedExpression(p) => extract_single_term(&p.expression, sign),
        _ => None,
    }
}

fn strip_unary_neg<'a>(expr: &'a Expression<'a>) -> (i8, &'a Expression<'a>) {
    match expr {
        Expression::UnaryExpression(un) if un.operator == UnaryOperator::UnaryNegation => {
            let (inner_sign, inner) = strip_unary_neg(&un.argument);
            (-inner_sign, inner)
        }
        Expression::ParenthesizedExpression(p) => strip_unary_neg(&p.expression),
        _ => (1, expr),
    }
}

fn extract_parseint_decoder_idx(expr: &Expression<'_>) -> Option<i32> {
    let call = match expr {
        Expression::CallExpression(c) => c,
        Expression::ParenthesizedExpression(p) => {
            if let Expression::CallExpression(c) = &p.expression {
                c
            } else {
                return None;
            }
        }
        _ => return None,
    };

    let is_parseint = match &call.callee {
        Expression::Identifier(id) => id.name.as_str() == "parseInt",
        _ => false,
    };
    if !is_parseint {
        return None;
    }

    let first_arg = call.arguments.first()?;
    if let Some(arg_expr) = first_arg.as_expression() {
        if let Expression::CallExpression(inner) = arg_expr {
            let idx_arg = inner.arguments.first()?;
            if let Some(idx_expr) = idx_arg.as_expression() {
                if let Expression::NumericLiteral(lit) = idx_expr {
                    return Some(lit.value as i32);
                }
            }
        }
    }
    None
}

fn brute_force_rotation(
    strings: &[String],
    terms: &[ChecksumTerm],
    target: f64,
    decoder_offset: i32,
    offset_op: OffsetOperation,
) -> Option<usize> {
    let len = strings.len();
    if len == 0 {
        return None;
    }

    let max_attempts = (len * 2).min(1_000_000);
    let mut rotated: Vec<String> = strings.to_vec();

    for attempt in 0..max_attempts {
        if let Some(checksum) = evaluate_checksum(&rotated, terms, decoder_offset, offset_op) {
            let matches = if target == target.floor() && checksum == checksum.floor() {
                (checksum as i64) == (target as i64)
            } else {
                (checksum - target).abs() < 0.5
            };

            if matches {
                return Some(attempt);
            }
        }

        if !rotated.is_empty() {
            let first = rotated.remove(0);
            rotated.push(first);
        }
    }

    None
}

fn evaluate_checksum(
    strings: &[String],
    terms: &[ChecksumTerm],
    decoder_offset: i32,
    offset_op: OffsetOperation,
) -> Option<f64> {
    let mut sum = 0.0f64;

    for term in terms {
        let mut product = 1.0f64;
        for (i, &idx) in term.indices.iter().enumerate() {
            let actual_index = match offset_op {
                OffsetOperation::None => idx as usize,
                OffsetOperation::Subtract => {
                    let result = idx - decoder_offset;
                    if result < 0 {
                        return None;
                    }
                    result as usize
                }
                OffsetOperation::Add => (idx + decoder_offset) as usize,
            };

            if actual_index >= strings.len() {
                return None;
            }

            let s = &strings[actual_index];
            let parsed = parse_int_js(s)?;
            let divisor = term.divisors[i];
            product *= parsed / divisor;
        }

        sum += f64::from(term.sign) * product;
    }

    Some(sum)
}

/// JS `parseInt()` semantics: parse leading digits, ignore trailing non-digits.
fn parse_int_js(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if s.starts_with("0x") || s.starts_with("0X") {
        let hex_str = &s[2..];
        let end = hex_str.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(hex_str.len());
        if end == 0 {
            return None;
        }
        return i64::from_str_radix(&hex_str[..end], 16).ok().map(|v| v as f64);
    }

    let (sign, digits_start) = if s.starts_with('-') {
        (-1.0, 1)
    } else if s.starts_with('+') {
        (1.0, 1)
    } else {
        (1.0, 0)
    };

    let rest = &s[digits_start..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if end == 0 {
        return None;
    }

    rest[..end].parse::<f64>().ok().map(|v| sign * v)
}

fn apply_rotation(strings: &mut Vec<String>, count: usize) {
    for _ in 0..count {
        if let Some(first) = strings.first().cloned() {
            strings.remove(0);
            strings.push(first);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::{DecoderInliner, DeobfuscateState, StringArrayInliner};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    fn run_self_defending(code: &str) -> (String, DeobfuscateState) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(ret.errors.is_empty(), "Parse errors: {:?}", ret.errors);
        let mut program = ret.program;

        let mut pass = SelfDefendingStringArray::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);
        oxc_traverse::traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);

        let mut state = DeobfuscateState::new();
        pass.finalize(&mut state);

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

        let mut decoder = DecoderInliner::new();
        oxc_traverse::traverse_mut_with_ctx(&mut decoder, &mut program, &mut ctx);

        let mut inliner = StringArrayInliner::new();
        oxc_traverse::traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);

        let output = Codegen::new().build(&program).code;
        let state = ctx.into_state();
        (output, state)
    }

    #[test]
    fn test_self_defending_fixture_zero_rotation() {
        let code = r#"
'use strict';
const dec = _0x5eba;
(function(arrFn, target) {
    const d = _0x5eba;
    const arr = arrFn();
    while (true) {
        try {
            const c = parseInt(d(0x100)) / 0x1
                    + parseInt(d(0x101)) / 0x2
                    - parseInt(d(0x102)) / 0x3;
            if (c === target) break;
            else arr.push(arr.shift());
        } catch (e) { arr.push(arr.shift()); }
    }
}(_0x3ad3, 180));
function getKey() { return dec(0x104); }
function _0x3ad3() {
    const arr = ["100", "200", "60", "https://abc@sentry.io/1",
                 "phc_PUBLIC_KEY_VALUE_HERE", "app.example.com:443", "extra-payload"];
    _0x3ad3 = function() { return arr; };
    return _0x3ad3();
}
function _0x5eba(idx, _key) {
    idx = idx - 0x100;
    const arr = _0x3ad3();
    return arr[idx];
}
"#;

        let (output, state) = run_self_defending(code);
        eprintln!("Output:\n{output}");

        assert!(
            state.string_arrays.contains_key("_0x3ad3"),
            "Should detect _0x3ad3 as string array wrapper"
        );

        assert!(
            state.decoders.contains_key("_0x5eba"),
            "Should detect _0x5eba as decoder"
        );

        assert!(
            output.contains("\"phc_PUBLIC_KEY_VALUE_HERE\""),
            "getKey() should contain the inlined PostHog key. Output:\n{output}"
        );
    }

    #[test]
    fn test_parse_int_js() {
        assert_eq!(parse_int_js("100"), Some(100.0));
        assert_eq!(parse_int_js("200"), Some(200.0));
        assert_eq!(parse_int_js("60"), Some(60.0));
        assert_eq!(parse_int_js("0xFF"), Some(255.0));
        assert_eq!(parse_int_js("123abc"), Some(123.0));
        assert_eq!(parse_int_js("abc"), None);
        assert_eq!(parse_int_js(""), None);
        assert_eq!(parse_int_js("-42"), Some(-42.0));
    }

    #[test]
    fn test_brute_force_rotation_zero() {
        let strings = vec![
            "100".into(),
            "200".into(),
            "60".into(),
            "sentry".into(),
            "phc_key".into(),
            "host".into(),
            "extra".into(),
        ];
        let terms = vec![
            ChecksumTerm {
                indices: vec![0x100],
                divisors: vec![1.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x101],
                divisors: vec![2.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x102],
                divisors: vec![3.0],
                sign: -1,
            },
        ];

        let result = brute_force_rotation(&strings, &terms, 180.0, 0x100, OffsetOperation::Subtract);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_brute_force_rotation_nonzero() {
        let strings = vec!["10".into(), "20".into(), "30".into(), "40".into()];
        let terms = vec![
            ChecksumTerm {
                indices: vec![0x100],
                divisors: vec![1.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x101],
                divisors: vec![2.0],
                sign: 1,
            },
        ];

        let result = brute_force_rotation(&strings, &terms, 35.0, 0x100, OffsetOperation::Subtract);
        assert_eq!(result, Some(1));
    }
}
