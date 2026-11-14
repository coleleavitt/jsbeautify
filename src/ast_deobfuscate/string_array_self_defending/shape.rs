//! Syntactic matching of the three pieces javascript-obfuscator emits for a
//! string array, resolved through oxc's symbols.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, BinaryOperator, BindingPattern,
    CallExpression, Expression, ExpressionStatement, Function, IdentifierReference, Statement, UnaryOperator,
    VariableDeclaration, WhileStatement,
};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::symbol::SymbolId;

use super::checksum::Checksum;
use crate::ast_deobfuscate::webpack::shapes::simple_params;

/// `function W() { const a = [...strings]; W = function () { return a; }; return W(); }`
pub(super) struct Wrapper {
    pub(super) symbol: SymbolId,
    pub(super) strings: Vec<String>,
    /// The self-assignment and the `W()` call inside the body.
    pub(super) own_refs: [ReferenceId; 2],
}

/// `function D(i, k) { i = i - OFFSET; const s = W(); let v = s[i]; return v; }`
/// (or `return s[i];`).
pub(super) struct Decoder {
    pub(super) symbol: SymbolId,
    /// The `W()` reference in the body.
    pub(super) wrapper_ref: ReferenceId,
    /// Added to a call's argument to get the array index.
    pub(super) offset: f64,
}

/// `(function (getArray, target) { const d = D, a = getArray(); while (!![]) {
/// try { const c = CHECKSUM; if (c === target) break; else a.push(a.shift()); }
/// catch (e) { a.push(a.shift()); } } }(W, TARGET));`
pub(super) struct Rotation {
    pub(super) wrapper_ref: ReferenceId,
    pub(super) target: f64,
    pub(super) checksum: Checksum,
}

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

pub(super) fn match_wrapper(func: &Function<'_>, scoping: &Scoping) -> Option<Wrapper> {
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
    if strings.is_empty() || scoping.get_resolved_reference_ids(array).len() != 1 {
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
    Some(Wrapper {
        symbol,
        strings,
        own_refs: [target.reference_id.get()?, self_call],
    })
}

pub(super) fn match_decoder(func: &Function<'_>, scoping: &Scoping, wrapper: SymbolId) -> Option<Decoder> {
    let symbol = func.id.as_ref()?.symbol_id.get()?;
    let params = simple_params(&func.params)?;
    if !is_plain(func) || !(1..=2).contains(&params.len()) {
        return None;
    }
    let index = params[0];
    let statements = func.body.as_ref()?.statements.as_slice();
    let [
        Statement::ExpressionStatement(shift),
        Statement::VariableDeclaration(array_decl),
        tail @ ..,
    ] = statements
    else {
        return None;
    };

    let offset = match_offset(&shift.expression, scoping, index)?;
    let (array, init) = single_declarator(array_decl)?;
    let wrapper_ref = call_without_args_of(init, scoping, wrapper)?;

    let is_read = |expr: &Expression<'_>| {
        matches!(expr, Expression::ComputedMemberExpression(member)
            if !member.optional
                && is_identifier_of(&member.object, scoping, array)
                && is_identifier_of(&member.expression, scoping, index))
    };
    let reads_element = match tail {
        [Statement::ReturnStatement(ret)] => ret.argument.as_ref().is_some_and(is_read),
        [
            Statement::VariableDeclaration(value_decl),
            Statement::ReturnStatement(ret),
        ] => single_declarator(value_decl).is_some_and(|(value, init)| {
            is_read(init)
                && ret
                    .argument
                    .as_ref()
                    .is_some_and(|arg| is_identifier_of(arg, scoping, value))
        }),
        _ => false,
    };
    reads_element.then_some(Decoder {
        symbol,
        wrapper_ref,
        offset,
    })
}

/// `i = i - N` or `i = i + N`, as the signed amount added to `i`.
fn match_offset(expr: &Expression<'_>, scoping: &Scoping, index: SymbolId) -> Option<f64> {
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
        BinaryOperator::Subtraction => Some(-amount.value),
        BinaryOperator::Addition => Some(amount.value),
        _ => None,
    }
}

pub(super) fn match_rotation(
    stmt: &ExpressionStatement<'_>,
    scoping: &Scoping,
    wrapper: SymbolId,
    decoder: SymbolId,
) -> Option<Rotation> {
    let Expression::CallExpression(call) = stmt.expression.without_parentheses() else {
        return None;
    };
    let Expression::FunctionExpression(func) = call.callee.without_parentheses() else {
        return None;
    };
    let [Argument::Identifier(wrapper_arg), Argument::NumericLiteral(target)] = call.arguments.as_slice() else {
        return None;
    };
    let params = simple_params(&func.params)?;
    let [get_array, target_param] = params.as_slice() else {
        return None;
    };
    if call.optional || !is_plain(func) || reference_symbol(scoping, wrapper_arg) != Some(wrapper) {
        return None;
    }

    let statements = func.body.as_ref()?.statements.as_slice();
    let [prelude @ .., Statement::WhileStatement(while_stmt)] = statements else {
        return None;
    };
    let mut decoders = vec![decoder];
    let mut array = None;
    for stmt in prelude {
        let Statement::VariableDeclaration(decl) = stmt else {
            return None;
        };
        for declarator in &decl.declarations {
            let symbol = binding_symbol(&declarator.id)?;
            let init = declarator.init.as_ref()?;
            if is_identifier_of(init, scoping, decoder) {
                decoders.push(symbol);
            } else if array.is_none() && call_without_args_of(init, scoping, *get_array).is_some() {
                array = Some(symbol);
            } else {
                return None;
            }
        }
    }
    let checksum = match_rotation_loop(while_stmt, scoping, array?, *target_param)?;

    Some(Rotation {
        wrapper_ref: wrapper_arg.reference_id.get()?,
        target: target.value,
        checksum: Checksum::compile(checksum, scoping, &decoders)?,
    })
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
