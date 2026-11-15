use oxc_ast::ast::{BinaryOperator, Expression, IdentifierReference, UnaryOperator};
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeId;
use rustc_hash::FxHashSet;

use super::Binding;

/// The largest integer a JS number holds exactly. Within it, `i64` and
/// double arithmetic agree.
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

fn safe(value: i64) -> Option<i64> {
    (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value).then_some(value)
}

/// An integer expression of literals and identifiers, `+`, `-` and `*`,
/// whose literals and intermediate values are all safe integers.
pub(super) enum ConstExpr {
    Lit(i64),
    /// A read of `Binding` at this source offset, in this scope.
    Read(Binding, u32, ScopeId),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
}

impl ConstExpr {
    pub(super) fn from_ast(expr: &Expression<'_>, scoping: &Scoping) -> Option<Self> {
        match expr {
            Expression::NumericLiteral(n) => {
                let v = n.value;
                if v.fract() != 0.0 || v.abs() > MAX_SAFE_INTEGER as f64 {
                    return None;
                }
                Some(Self::Lit(v as i64))
            }
            Expression::Identifier(id) => Some(Self::Read(
                Binding::of(id, scoping),
                id.span().start,
                scoping.get_reference(id.reference_id()).scope_id(),
            )),
            Expression::BinaryExpression(bin) => {
                let l = Box::new(Self::from_ast(&bin.left, scoping)?);
                let r = Box::new(Self::from_ast(&bin.right, scoping)?);
                match bin.operator {
                    BinaryOperator::Addition => Some(Self::Add(l, r)),
                    BinaryOperator::Subtraction => Some(Self::Sub(l, r)),
                    BinaryOperator::Multiplication => Some(Self::Mul(l, r)),
                    _ => None,
                }
            }
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => {
                Self::from_ast(&u.argument, scoping)
            }
            Expression::ParenthesizedExpression(p) => Self::from_ast(&p.expression, scoping),
            _ => None,
        }
    }

    /// The value, given what `read` returns for a binding read at an offset in a scope.
    pub(super) fn eval(&self, read: &impl Fn(&Binding, u32, ScopeId) -> Option<usize>) -> Option<i64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Read(binding, at, scope) => read(binding, *at, *scope).and_then(|v| i64::try_from(v).ok()),
            Self::Add(l, r) => l.eval(read)?.checked_add(r.eval(read)?).and_then(safe),
            Self::Sub(l, r) => l.eval(read)?.checked_sub(r.eval(read)?).and_then(safe),
            Self::Mul(l, r) => l.eval(read)?.checked_mul(r.eval(read)?).and_then(safe),
        }
    }
}

impl ConstExpr {
    /// Whether evaluating it runs no code: every read is of a binding in `numbers`.
    pub(super) fn reads_only(&self, numbers: &FxHashSet<Binding>) -> bool {
        match self {
            Self::Lit(_) => true,
            Self::Read(binding, ..) => numbers.contains(binding),
            Self::Add(l, r) | Self::Sub(l, r) | Self::Mul(l, r) => l.reads_only(numbers) && r.reads_only(numbers),
        }
    }
}

impl Binding {
    /// What `id` reads: its resolved binding, or the global of its name.
    pub(super) fn of(id: &IdentifierReference<'_>, scoping: &Scoping) -> Self {
        scoping
            .get_reference(id.reference_id())
            .symbol_id()
            .map_or_else(|| Self::Global(id.name.as_str().to_string()), Self::Symbol)
    }
}
