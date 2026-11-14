//! Static evaluation of the rotation IIFE's checksum.
//!
//! The IIFE rotates the array with `push(shift())` until a sum of
//! `parseInt(decoder(N))` terms equals its target. The checksum is compiled
//! once to a postfix program over `f64`, the arithmetic JavaScript performs
//! on numbers, and evaluated for each rotation without running JavaScript.

use oxc_ast::ast::{Argument, BinaryOperator, CallExpression, Expression, UnaryOperator};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;

use super::shape::reference_symbol;

/// Upper bound on the compiled checksum's length.
const MAX_OPS: usize = 4_096;
/// Upper bound on the checksum's nesting depth.
const MAX_DEPTH: usize = 256;
/// Upper bound on `ops × array length`, the work of a full search.
const MAX_WORK: usize = 50_000_000;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Number(f64),
    /// `parseInt(decoder(arg))`: pushes the parsed element at array index
    /// `arg + offset`, or `NaN` when that index reads `undefined`.
    Decode(f64),
    Negate,
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug)]
pub(super) struct Checksum {
    ops: Vec<Op>,
}

impl Checksum {
    /// Compiles `expr` when it only uses number literals, unary `-`/`+`,
    /// binary `+ - * /`, and `parseInt(decoder(<index literal>))` with the
    /// global `parseInt` and a decoder symbol from `decoders`. String index
    /// literals are accepted when the decoder converts them to numbers.
    pub(super) fn compile(
        expr: &Expression<'_>,
        scoping: &Scoping,
        decoders: &[SymbolId],
        coerces_strings: bool,
    ) -> Option<Self> {
        let mut compiler = Compiler {
            scoping,
            decoders,
            coerces_strings,
            ops: Vec::new(),
        };
        compiler.compile(expr, 0)?;
        Some(Self { ops: compiler.ops })
    }

    /// The number of `push(shift())` steps after which the checksum first
    /// equals `target`, or `None` when no rotation matches (the IIFE would
    /// loop forever), the search is too large, or a term cannot be evaluated
    /// exactly.
    pub(super) fn solve(&self, strings: &[String], offset: f64, target: f64) -> Option<usize> {
        let len = strings.len();
        if len == 0 || self.ops.len().saturating_mul(len) > MAX_WORK {
            return None;
        }
        let parsed: Vec<Option<f64>> = strings.iter().map(|s| parse_int(s)).collect();
        let mut stack = Vec::with_capacity(self.ops.len());
        (0..len).find_map(|rotation| {
            let value = self.evaluate(&parsed, offset, rotation, &mut stack);
            match value {
                Ok(value) if value == target => Some(Some(rotation)),
                Ok(_) => None,
                Err(()) => Some(None),
            }
        })?
    }

    fn evaluate(&self, parsed: &[Option<f64>], offset: f64, rotation: usize, stack: &mut Vec<f64>) -> Result<f64, ()> {
        stack.clear();
        for op in &self.ops {
            let value = match *op {
                Op::Number(value) => value,
                Op::Decode(arg) => match array_index(arg + offset, parsed.len()) {
                    Some(index) => parsed[(index + rotation) % parsed.len()].ok_or(())?,
                    None => f64::NAN,
                },
                Op::Negate => -stack.pop().ok_or(())?,
                binary => {
                    let right = stack.pop().ok_or(())?;
                    let left = stack.pop().ok_or(())?;
                    match binary {
                        Op::Add => left + right,
                        Op::Subtract => left - right,
                        Op::Multiply => left * right,
                        _ => left / right,
                    }
                }
            };
            stack.push(value);
        }
        match stack.as_slice() {
            [value] => Ok(*value),
            _ => Err(()),
        }
    }
}

/// The array index JavaScript reads for the number `index`, when it is an
/// integer in `0..len`.
pub(super) fn array_index(index: f64, len: usize) -> Option<usize> {
    if index.fract() != 0.0 || index < 0.0 || index >= len as f64 {
        return None;
    }
    Some(index as usize)
}

struct Compiler<'s> {
    scoping: &'s Scoping,
    decoders: &'s [SymbolId],
    coerces_strings: bool,
    ops: Vec<Op>,
}

impl Compiler<'_> {
    fn compile(&mut self, expr: &Expression<'_>, depth: usize) -> Option<()> {
        if self.ops.len() >= MAX_OPS || depth >= MAX_DEPTH {
            return None;
        }
        let depth = depth + 1;
        match expr {
            Expression::NumericLiteral(lit) => self.ops.push(Op::Number(lit.value)),
            Expression::ParenthesizedExpression(paren) => self.compile(&paren.expression, depth)?,
            Expression::UnaryExpression(unary) => {
                self.compile(&unary.argument, depth)?;
                match unary.operator {
                    UnaryOperator::UnaryNegation => self.ops.push(Op::Negate),
                    // Every compiled value is already a number, so `+` keeps it.
                    UnaryOperator::UnaryPlus => {}
                    _ => return None,
                }
            }
            Expression::BinaryExpression(binary) => {
                let op = match binary.operator {
                    BinaryOperator::Addition => Op::Add,
                    BinaryOperator::Subtraction => Op::Subtract,
                    BinaryOperator::Multiplication => Op::Multiply,
                    BinaryOperator::Division => Op::Divide,
                    _ => return None,
                };
                self.compile(&binary.left, depth)?;
                self.compile(&binary.right, depth)?;
                self.ops.push(op);
            }
            Expression::CallExpression(call) => {
                let index = self.parse_int_of_decoder_call(call)?;
                self.ops.push(Op::Decode(index));
            }
            _ => return None,
        }
        Some(())
    }

    /// The index literal of `parseInt(decoder(<index literal>))`, as a number.
    fn parse_int_of_decoder_call(&self, call: &CallExpression<'_>) -> Option<f64> {
        let Expression::Identifier(parse_int) = &call.callee else {
            return None;
        };
        let parse_int_ref = parse_int.reference_id.get()?;
        if call.optional
            || parse_int.name != "parseInt"
            || self.scoping.get_reference(parse_int_ref).symbol_id().is_some()
        {
            return None;
        }
        let [Argument::CallExpression(inner)] = call.arguments.as_slice() else {
            return None;
        };
        let Expression::Identifier(decoder) = &inner.callee else {
            return None;
        };
        if inner.optional || !self.decoders.contains(&reference_symbol(self.scoping, decoder)?) {
            return None;
        }
        index_literal(inner.arguments.as_slice(), self.coerces_strings)
    }
}

/// The single number argument of a decoder call, or a string argument that
/// the decoder converts to one.
pub(super) fn index_literal(args: &[Argument<'_>], coerces_strings: bool) -> Option<f64> {
    match args {
        [Argument::NumericLiteral(lit)] => Some(lit.value),
        [Argument::StringLiteral(lit)] if coerces_strings => string_to_number(&lit.value),
        _ => None,
    }
}

/// JavaScript's `parseInt(s)` without a radix. `Some(NaN)` when `s` has no
/// leading digits; `None` when the result is not computed exactly here.
pub(super) fn parse_int(s: &str) -> Option<f64> {
    let s = s.trim_start_matches(is_js_whitespace);
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, s) = match s.get(..2) {
        Some("0x" | "0X") => (16, &s[2..]),
        _ => (10, s),
    };
    let digits = s.find(|c: char| !c.is_digit(radix)).map_or(s, |end| &s[..end]);
    if digits.is_empty() {
        return Some(f64::NAN);
    }
    let value = if radix == 16 {
        // Hex digits beyond 2^53 would need rounding the way engines do it.
        let value = u64::from_str_radix(digits, 16).ok()?;
        if value > (1 << 53) {
            return None;
        }
        value as f64
    } else {
        // Engines may approximate past 20 significant digits.
        if digits.trim_start_matches('0').len() > 20 {
            return None;
        }
        digits.parse::<f64>().ok()?
    };
    Some(if negative { -value } else { value })
}

/// JavaScript's `ToNumber` of a string, for decimal or `0x` hex digits
/// between optional whitespace (an empty string is 0). `None` for any
/// other form.
pub(super) fn string_to_number(s: &str) -> Option<f64> {
    let s = s.trim_matches(is_js_whitespace);
    if s.is_empty() {
        return Some(0.0);
    }
    let (radix, digits) = match s.get(..2) {
        Some("0x" | "0X") => (16, &s[2..]),
        _ => (10, s),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let value = u64::from_str_radix(digits, radix).ok()?;
    (value <= 1 << 53).then_some(value as f64)
}

/// ECMAScript `WhiteSpace` and `LineTerminator`, which `parseInt` skips.
const fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}
