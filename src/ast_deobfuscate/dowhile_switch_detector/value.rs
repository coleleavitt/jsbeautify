//! The values a do-while dispatcher's state takes, computed the way
//! JavaScript computes them.

use oxc_ast::ast::{BinaryOperator, Expression, IdentifierReference, UnaryOperator};

/// Largest magnitude whose integers `f64` arithmetic keeps exact.
const MAX_EXACT: u64 = 1 << 53;

/// Longest string a constant may hold. Doubling a string per binding
/// (`b = a + a; c = b + b; …`) must not exhaust memory.
const MAX_STRING_LEN: usize = 64;

/// A primitive a dispatcher's state or case label holds. Two values are
/// `===` exactly when they are equal here: integers are kept only within
/// `±2^53`, where `f64` represents every one of them, and `-0` is `0`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateValue {
    Number(i64),
    String(Box<str>),
}

/// An evaluated operand. An array literal is an object, never a state
/// value, but `[1] + [2]` and `+[5]` read the string it converts to.
enum Operand {
    Value(StateValue),
    Array(Box<str>),
}

/// The value `expr` evaluates to, when it is built from integer literals,
/// one-element or empty array literals, unary `+`, binary `+`, `-` and `*`,
/// and identifiers `lookup` resolves. Evaluating such an expression has no
/// side effects.
pub fn evaluate(
    expr: &Expression<'_>,
    lookup: &impl Fn(&IdentifierReference<'_>) -> Option<StateValue>,
) -> Option<StateValue> {
    match operand(expr, lookup)? {
        Operand::Value(value) => Some(value),
        Operand::Array(_) => None,
    }
}

/// `left + right`.
pub fn add(left: &StateValue, right: &StateValue) -> Option<StateValue> {
    add_operands(&Operand::Value(left.clone()), &Operand::Value(right.clone()))
}

/// `left - right`.
pub fn subtract(left: &StateValue, right: &StateValue) -> Option<StateValue> {
    arithmetic(
        &Operand::Value(left.clone()),
        &Operand::Value(right.clone()),
        i64::checked_sub,
    )
}

fn operand(expr: &Expression<'_>, lookup: &impl Fn(&IdentifierReference<'_>) -> Option<StateValue>) -> Option<Operand> {
    match expr {
        Expression::NumericLiteral(literal) => integer(literal.value).map(Operand::Value),
        Expression::Identifier(id) => lookup(id).map(Operand::Value),
        Expression::ParenthesizedExpression(paren) => operand(&paren.expression, lookup),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryPlus => {
            to_number(&operand(&unary.argument, lookup)?).map(|n| Operand::Value(StateValue::Number(n)))
        }
        Expression::BinaryExpression(binary) => {
            let left = operand(&binary.left, lookup)?;
            let right = operand(&binary.right, lookup)?;
            match binary.operator {
                BinaryOperator::Addition => add_operands(&left, &right),
                BinaryOperator::Subtraction => arithmetic(&left, &right, i64::checked_sub),
                BinaryOperator::Multiplication => arithmetic(&left, &right, i64::checked_mul),
                _ => None,
            }
            .map(Operand::Value)
        }
        Expression::ArrayExpression(array) => match array.elements.as_slice() {
            [] => Some(Operand::Array("".into())),
            [element] => {
                let element = operand(element.as_expression()?, lookup)?;
                Some(Operand::Array(to_string(&element)))
            }
            _ => None,
        },
        _ => None,
    }
}

/// JavaScript `+`: string concatenation when either side converts to a
/// string (an array converts to one), numeric addition otherwise.
fn add_operands(left: &Operand, right: &Operand) -> Option<StateValue> {
    if let (Operand::Value(StateValue::Number(l)), Operand::Value(StateValue::Number(r))) = (left, right) {
        return number(l.checked_add(*r)?);
    }
    let joined = format!("{}{}", to_string(left), to_string(right));
    (joined.len() <= MAX_STRING_LEN).then(|| StateValue::String(joined.into()))
}

fn arithmetic(left: &Operand, right: &Operand, op: fn(i64, i64) -> Option<i64>) -> Option<StateValue> {
    number(op(to_number(left)?, to_number(right)?)?)
}

fn number(n: i64) -> Option<StateValue> {
    (n.unsigned_abs() <= MAX_EXACT).then_some(StateValue::Number(n))
}

fn integer(value: f64) -> Option<StateValue> {
    (value.fract() == 0.0 && value.abs() <= MAX_EXACT as f64).then_some(StateValue::Number(value as i64))
}

fn to_string(operand: &Operand) -> Box<str> {
    match operand {
        Operand::Value(StateValue::Number(n)) => n.to_string().into(),
        Operand::Value(StateValue::String(s)) | Operand::Array(s) => s.clone(),
    }
}

/// `ToNumber`, for the integer strings this module produces: an empty
/// string is `0`, and a string other than an optional sign followed by
/// ASCII digits is not evaluated.
fn to_number(operand: &Operand) -> Option<i64> {
    let s = match operand {
        Operand::Value(StateValue::Number(n)) => return Some(*n),
        Operand::Value(StateValue::String(s)) | Operand::Array(s) => s,
    };
    if s.is_empty() {
        return Some(0);
    }
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    (n.unsigned_abs() <= MAX_EXACT).then_some(n)
}
