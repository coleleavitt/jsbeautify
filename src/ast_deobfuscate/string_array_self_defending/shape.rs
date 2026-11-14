//! Syntactic matching of the three pieces javascript-obfuscator emits for a
//! string array, resolved through oxc's symbols.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, BinaryOperator, BindingPattern,
    CallExpression, Expression, ExpressionStatement, Function, IdentifierReference, SimpleAssignmentTarget, Statement,
    UnaryOperator, UpdateOperator, VariableDeclaration, VariableDeclarationKind, WhileStatement,
};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::symbol::SymbolId;

use super::checksum::Checksum;
use crate::ast_deobfuscate::webpack::shapes::simple_params;

/// The string array: a self-replacing wrapper function, `function W() {
/// const a = [...]; W = function () { return a; }; return W(); }`, or a
/// top-level `var a = [...]`.
pub(super) struct StringArray {
    pub(super) symbol: SymbolId,
    pub(super) strings: Vec<String>,
    /// The wrapper's self-assignment and self-call.
    pub(super) own_refs: Vec<ReferenceId>,
    /// The program-body index of a variable's declaration; `None` for a
    /// wrapper function, which is hoisted.
    pub(super) declared_at: Option<usize>,
}

/// `function D(i, k) { i = i - OFFSET; const s = W(); let v = s[i]; return v; }`,
/// or `var D = function (i) { i = i - OFFSET; var v = a[i]; return v; }`; the
/// offset is optional, and `return s[i];` may replace the temporary.
pub(super) struct Decoder {
    pub(super) symbol: SymbolId,
    /// The reference to the array (or its wrapper) in the body.
    pub(super) array_ref: ReferenceId,
    /// Added to a call's argument to get the array index.
    pub(super) offset: f64,
    /// `i = i - OFFSET` converts a string argument to a number.
    pub(super) coerces_strings: bool,
    /// The program-body index of a variable's declaration; `None` for a
    /// function declaration.
    pub(super) declared_at: Option<usize>,
}

pub(super) struct Rotation {
    /// The array (or wrapper) passed to the IIFE.
    pub(super) array_ref: ReferenceId,
    pub(super) kind: RotationKind,
}

pub(super) enum RotationKind {
    /// `(function (getArray, target) { const d = D, a = getArray(); while (!![]) {
    /// try { const c = CHECKSUM; if (c === target) break; else a.push(a.shift()); }
    /// catch (e) { a.push(a.shift()); } } }(W, TARGET));`
    Checksum { checksum: Checksum, target: f64 },
    /// `(function (a, n) { var f = function (k) { while (--k) { a.push(a.shift()); } };
    /// f(++n); }(A, N));`, with the counter `f` starts from.
    Countdown(f64),
}

impl Rotation {
    /// The number of `push(shift())` steps it performs before returning.
    /// A checksum search spends `work_budget`.
    pub(super) fn steps(&self, strings: &[String], offset: f64, work_budget: &mut usize) -> Option<usize> {
        match &self.kind {
            RotationKind::Checksum { checksum, target } => checksum.solve(strings, offset, *target, work_budget),
            // `while (--k)` runs `k - 1` times, and forever unless `k` is a
            // positive integer. A count past `MAX_COUNTDOWN` would keep the
            // input spinning for so long that the output must spin too.
            RotationKind::Countdown(start) => {
                if start.fract() != 0.0 || *start < 1.0 || *start > MAX_COUNTDOWN {
                    return None;
                }
                let len = u64::try_from(strings.len()).ok().filter(|&len| len > 0)?;
                usize::try_from((*start as u64 - 1) % len).ok()
            }
        }
    }
}

/// The largest countdown solved: 2^32 steps.
const MAX_COUNTDOWN: f64 = 4_294_967_296.0;

pub(super) fn reference_symbol(scoping: &Scoping, ident: &IdentifierReference<'_>) -> Option<SymbolId> {
    scoping.get_reference(ident.reference_id.get()?).symbol_id()
}

fn binding_symbol(pattern: &BindingPattern<'_>) -> Option<SymbolId> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
        _ => None,
    }
}

fn is_identifier_of(expr: &Expression<'_>, scoping: &Scoping, symbol: SymbolId) -> bool {
    matches!(expr, Expression::Identifier(id) if reference_symbol(scoping, id) == Some(symbol))
}

/// `callee()` with no arguments, its callee a reference to `symbol`.
fn call_without_args_of(expr: &Expression<'_>, scoping: &Scoping, symbol: SymbolId) -> Option<ReferenceId> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return None;
    };
    (!call.optional && call.arguments.is_empty() && reference_symbol(scoping, callee) == Some(symbol))
        .then(|| callee.reference_id.get())
        .flatten()
}

const fn is_plain(func: &Function<'_>) -> bool {
    !func.r#async && !func.generator
}

/// The single declarator `name = init` of `decl`.
fn single_declarator<'d, 'a>(decl: &'d VariableDeclaration<'a>) -> Option<(SymbolId, &'d Expression<'a>)> {
    let [declarator] = decl.declarations.as_slice() else {
        return None;
    };
    Some((binding_symbol(&declarator.id)?, declarator.init.as_ref()?))
}

/// The strings of an array literal holding only string literals.
fn string_elements(init: &Expression<'_>) -> Option<Vec<String>> {
    let Expression::ArrayExpression(elements) = init else {
        return None;
    };
    let strings = elements
        .elements
        .iter()
        .map(|element| match element {
            ArrayExpressionElement::StringLiteral(lit) if !lit.lone_surrogates => Some(lit.value.to_string()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!strings.is_empty()).then_some(strings)
}

pub(super) fn match_wrapper(func: &Function<'_>, scoping: &Scoping) -> Option<StringArray> {
    let symbol = func.id.as_ref()?.symbol_id.get()?;
    if !is_plain(func) || !simple_params(&func.params)?.is_empty() {
        return None;
    }
    let [
        Statement::VariableDeclaration(array_decl),
        Statement::ExpressionStatement(reassign),
        Statement::ReturnStatement(ret),
    ] = func.body.as_ref()?.statements.as_slice()
    else {
        return None;
    };

    let (array, init) = single_declarator(array_decl)?;
    let strings = string_elements(init)?;
    if scoping.get_resolved_reference_ids(array).len() != 1 {
        return None;
    }

    let Expression::AssignmentExpression(assign) = &reassign.expression else {
        return None;
    };
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    let Expression::FunctionExpression(getter) = &assign.right else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign
        || reference_symbol(scoping, target) != Some(symbol)
        || !is_plain(getter)
        || !simple_params(&getter.params)?.is_empty()
    {
        return None;
    }
    let [Statement::ReturnStatement(getter_ret)] = getter.body.as_ref()?.statements.as_slice() else {
        return None;
    };
    if !getter_ret
        .argument
        .as_ref()
        .is_some_and(|arg| is_identifier_of(arg, scoping, array))
    {
        return None;
    }

    let self_call = call_without_args_of(ret.argument.as_ref()?, scoping, symbol)?;
    Some(StringArray {
        symbol,
        strings,
        own_refs: vec![target.reference_id.get()?, self_call],
        declared_at: None,
    })
}

/// `var a = [...strings]` (or `let`/`const`) at program-body index `index`.
pub(super) fn match_array_variable(decl: &VariableDeclaration<'_>, index: usize) -> Option<StringArray> {
    if !matches!(
        decl.kind,
        VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
    ) {
        return None;
    }
    let (symbol, init) = single_declarator(decl)?;
    Some(StringArray {
        symbol,
        strings: string_elements(init)?,
        own_refs: Vec::new(),
        declared_at: Some(index),
    })
}

/// A function that may be a decoder: a function declaration, or the only
/// declarator of a variable declaration initialised to a function expression.
pub(super) struct DecoderCandidate<'p, 'a> {
    pub(super) symbol: SymbolId,
    pub(super) func: &'p Function<'a>,
    pub(super) declared_at: Option<usize>,
}

pub(super) fn decoder_candidate<'p, 'a>(stmt: &'p Statement<'a>, index: usize) -> Option<DecoderCandidate<'p, 'a>> {
    match stmt {
        Statement::FunctionDeclaration(func) => Some(DecoderCandidate {
            symbol: func.id.as_ref()?.symbol_id.get()?,
            func,
            declared_at: None,
        }),
        Statement::VariableDeclaration(decl)
            if matches!(
                decl.kind,
                VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
            ) =>
        {
            let (symbol, Expression::FunctionExpression(func)) = single_declarator(decl)? else {
                return None;
            };
            Some(DecoderCandidate {
                symbol,
                func,
                declared_at: Some(index),
            })
        }
        _ => None,
    }
}

pub(super) fn match_decoder(
    candidate: &DecoderCandidate<'_, '_>,
    scoping: &Scoping,
    array: &StringArray,
) -> Option<Decoder> {
    let func = candidate.func;
    let params = simple_params(&func.params)?;
    if !is_plain(func) || !(1..=2).contains(&params.len()) {
        return None;
    }
    let index = params[0];
    let mut statements = func.body.as_ref()?.statements.as_slice();

    let (offset, coerces_strings) = match statements {
        [Statement::ExpressionStatement(shift), rest @ ..] => {
            statements = rest;
            match_offset(&shift.expression, scoping, index)?
        }
        _ => (0.0, false),
    };
    // A wrapper is called into a local; a variable is read directly.
    let (source, wrapper_call) = if array.declared_at.is_none() {
        let [Statement::VariableDeclaration(array_decl), rest @ ..] = statements else {
            return None;
        };
        statements = rest;
        let (local, init) = single_declarator(array_decl)?;
        (local, Some(call_without_args_of(init, scoping, array.symbol)?))
    } else {
        (array.symbol, None)
    };

    let read_ref = |expr: &Expression<'_>| match expr {
        Expression::ComputedMemberExpression(member)
            if !member.optional && is_identifier_of(&member.expression, scoping, index) =>
        {
            match &member.object {
                Expression::Identifier(object) if reference_symbol(scoping, object) == Some(source) => {
                    object.reference_id.get()
                }
                _ => None,
            }
        }
        _ => None,
    };
    let read = match statements {
        [Statement::ReturnStatement(ret)] => read_ref(ret.argument.as_ref()?)?,
        [
            Statement::VariableDeclaration(value_decl),
            Statement::ReturnStatement(ret),
        ] => {
            let (value, init) = single_declarator(value_decl)?;
            if !ret
                .argument
                .as_ref()
                .is_some_and(|arg| is_identifier_of(arg, scoping, value))
            {
                return None;
            }
            read_ref(init)?
        }
        _ => return None,
    };
    Some(Decoder {
        symbol: candidate.symbol,
        array_ref: wrapper_call.unwrap_or(read),
        offset,
        coerces_strings,
        declared_at: candidate.declared_at,
    })
}

/// `i = i - N` or `i = i + N`, as the signed amount added to `i` and whether
/// the operation converts a string `i` to a number.
fn match_offset(expr: &Expression<'_>, scoping: &Scoping, index: SymbolId) -> Option<(f64, bool)> {
    let Expression::AssignmentExpression(assign) = expr else {
        return None;
    };
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    let Expression::BinaryExpression(binary) = &assign.right else {
        return None;
    };
    let Expression::NumericLiteral(amount) = &binary.right else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign
        || reference_symbol(scoping, target) != Some(index)
        || !is_identifier_of(&binary.left, scoping, index)
    {
        return None;
    }
    match binary.operator {
        BinaryOperator::Subtraction => Some((-amount.value, true)),
        BinaryOperator::Addition => Some((amount.value, false)),
        _ => None,
    }
}

pub(super) fn match_rotation(
    stmt: &ExpressionStatement<'_>,
    scoping: &Scoping,
    array: &StringArray,
    decoder: &Decoder,
) -> Option<Rotation> {
    let Expression::CallExpression(call) = stmt.expression.without_parentheses() else {
        return None;
    };
    let Expression::FunctionExpression(func) = call.callee.without_parentheses() else {
        return None;
    };
    let [Argument::Identifier(array_arg), Argument::NumericLiteral(number)] = call.arguments.as_slice() else {
        return None;
    };
    let params = simple_params(&func.params)?;
    let [array_param, number_param] = params.as_slice() else {
        return None;
    };
    if call.optional || !is_plain(func) || reference_symbol(scoping, array_arg) != Some(array.symbol) {
        return None;
    }
    let statements = func.body.as_ref()?.statements.as_slice();
    let kind = if array.declared_at.is_none() {
        match_checksum_body(statements, scoping, *array_param, *number_param, number.value, decoder)?
    } else {
        match_countdown_body(statements, scoping, *array_param, *number_param, number.value)?
    };
    Some(Rotation {
        array_ref: array_arg.reference_id.get()?,
        kind,
    })
}

/// `const d = D, a = getArray(); while (...) {...}`: the checksum loop over
/// the array the wrapper returns.
fn match_checksum_body(
    statements: &[Statement<'_>],
    scoping: &Scoping,
    get_array: SymbolId,
    target_param: SymbolId,
    target: f64,
    decoder: &Decoder,
) -> Option<RotationKind> {
    let [prelude @ .., Statement::WhileStatement(while_stmt)] = statements else {
        return None;
    };
    let mut decoders = vec![decoder.symbol];
    let mut array = None;
    for stmt in prelude {
        let Statement::VariableDeclaration(decl) = stmt else {
            return None;
        };
        for declarator in &decl.declarations {
            let symbol = binding_symbol(&declarator.id)?;
            let init = declarator.init.as_ref()?;
            if is_identifier_of(init, scoping, decoder.symbol) {
                decoders.push(symbol);
            } else if array.is_none() && call_without_args_of(init, scoping, get_array).is_some() {
                array = Some(symbol);
            } else {
                return None;
            }
        }
    }
    let checksum = match_rotation_loop(while_stmt, scoping, array?, target_param)?;
    Some(RotationKind::Checksum {
        checksum: Checksum::compile(checksum, scoping, &decoders, decoder.coerces_strings)?,
        target,
    })
}

/// `var f = function (k) { while (--k) { a.push(a.shift()); } }; f(++n);`,
/// or `f(<number literal>)`, with `n` bound to `number`.
fn match_countdown_body(
    statements: &[Statement<'_>],
    scoping: &Scoping,
    array: SymbolId,
    counter_param: SymbolId,
    number: f64,
) -> Option<RotationKind> {
    let [
        Statement::VariableDeclaration(decl),
        Statement::ExpressionStatement(run),
    ] = statements
    else {
        return None;
    };
    let (rotate, Expression::FunctionExpression(func)) = single_declarator(decl)? else {
        return None;
    };
    let params = simple_params(&func.params)?;
    let [counter] = params.as_slice() else {
        return None;
    };
    let [Statement::WhileStatement(while_stmt)] = func.body.as_ref()?.statements.as_slice() else {
        return None;
    };
    let decrements_counter = matches!(&while_stmt.test, Expression::UpdateExpression(update)
        if update.prefix
            && update.operator == UpdateOperator::Decrement
            && matches!(&update.argument, SimpleAssignmentTarget::AssignmentTargetIdentifier(id)
                if reference_symbol(scoping, id) == Some(*counter)));
    if !is_plain(func) || !decrements_counter || !is_rotate_step(std::slice::from_ref(&while_stmt.body), scoping, array)
    {
        return None;
    }

    let Expression::CallExpression(call) = &run.expression else {
        return None;
    };
    let [arg] = call.arguments.as_slice() else {
        return None;
    };
    if call.optional || !is_identifier_of(&call.callee, scoping, rotate) {
        return None;
    }
    let start = match arg {
        Argument::NumericLiteral(lit) => lit.value,
        Argument::UpdateExpression(update)
            if update.prefix
                && update.operator == UpdateOperator::Increment
                && matches!(&update.argument, SimpleAssignmentTarget::AssignmentTargetIdentifier(id)
                    if reference_symbol(scoping, id) == Some(counter_param)) =>
        {
            number + 1.0
        }
        _ => return None,
    };
    Some(RotationKind::Countdown(start))
}

/// The checksum expression of `while (!![]) { try { const c = CHECKSUM; if (c
/// === target) break; else a.push(a.shift()); } catch (e) { a.push(a.shift()); } }`.
fn match_rotation_loop<'w, 'a>(
    while_stmt: &'w WhileStatement<'a>,
    scoping: &Scoping,
    array: SymbolId,
    target: SymbolId,
) -> Option<&'w Expression<'a>> {
    if !is_always_true(&while_stmt.test) {
        return None;
    }

    let try_stmt = match &while_stmt.body {
        Statement::TryStatement(try_stmt) => try_stmt,
        Statement::BlockStatement(block) => match block.body.as_slice() {
            [Statement::TryStatement(try_stmt)] => try_stmt,
            _ => return None,
        },
        _ => return None,
    };
    let handler = try_stmt.handler.as_ref()?;
    if try_stmt.finalizer.is_some() || !is_rotate_step(handler.body.body.as_slice(), scoping, array) {
        return None;
    }
    let [Statement::VariableDeclaration(sum_decl), Statement::IfStatement(check)] = try_stmt.block.body.as_slice()
    else {
        return None;
    };
    let (sum, checksum) = single_declarator(sum_decl)?;
    let Expression::BinaryExpression(compare) = &check.test else {
        return None;
    };
    let compares_sum = (is_identifier_of(&compare.left, scoping, sum)
        && is_identifier_of(&compare.right, scoping, target))
        || (is_identifier_of(&compare.left, scoping, target) && is_identifier_of(&compare.right, scoping, sum));
    if compare.operator != BinaryOperator::StrictEquality
        || !compares_sum
        || !is_unlabeled_break(&check.consequent)
        || !check
            .alternate
            .as_ref()
            .is_some_and(|alternate| is_rotate_step(std::slice::from_ref(alternate), scoping, array))
    {
        return None;
    }

    Some(checksum)
}

/// `true` or `!![]`.
fn is_always_true(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::BooleanLiteral(lit) => lit.value,
        Expression::UnaryExpression(outer) if outer.operator == UnaryOperator::LogicalNot => {
            matches!(&outer.argument, Expression::UnaryExpression(inner)
                if inner.operator == UnaryOperator::LogicalNot
                    && matches!(&inner.argument, Expression::ArrayExpression(arr) if arr.elements.is_empty()))
        }
        _ => false,
    }
}

fn is_unlabeled_break(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::BreakStatement(brk) => brk.label.is_none(),
        Statement::BlockStatement(block) => {
            matches!(block.body.as_slice(), [Statement::BreakStatement(brk)] if brk.label.is_none())
        }
        _ => false,
    }
}

/// `[a.push(a.shift())]`, either directly or as the only statement of a block.
fn is_rotate_step(stmts: &[Statement<'_>], scoping: &Scoping, array: SymbolId) -> bool {
    let stmt = match stmts {
        [Statement::BlockStatement(block)] => match block.body.as_slice() {
            [stmt] => stmt,
            _ => return false,
        },
        [stmt] => stmt,
        _ => return false,
    };
    let Statement::ExpressionStatement(expr_stmt) = stmt else {
        return false;
    };
    let Expression::CallExpression(push) = &expr_stmt.expression else {
        return false;
    };
    let [Argument::CallExpression(shift)] = push.arguments.as_slice() else {
        return false;
    };
    is_method_call(push, scoping, array, "push")
        && shift.arguments.is_empty()
        && is_method_call(shift, scoping, array, "shift")
}

/// `array.method(...)` or `array['method'](...)`, not optional.
fn is_method_call(call: &CallExpression<'_>, scoping: &Scoping, array: SymbolId, method: &str) -> bool {
    if call.optional {
        return false;
    }
    match &call.callee {
        Expression::StaticMemberExpression(member) => {
            !member.optional && member.property.name == method && is_identifier_of(&member.object, scoping, array)
        }
        Expression::ComputedMemberExpression(member) => {
            !member.optional
                && matches!(&member.expression, Expression::StringLiteral(lit) if lit.value == method)
                && is_identifier_of(&member.object, scoping, array)
        }
        _ => false,
    }
}
