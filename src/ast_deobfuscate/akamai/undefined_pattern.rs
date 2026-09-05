use oxc_ast::ast::{Expression, IdentifierReference};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

pub struct UndefinedPatternNormalizer {
    replaced: usize,
}

impl UndefinedPatternNormalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self { replaced: 0 }
    }

    #[must_use]
    pub const fn replaced_count(&self) -> usize {
        self.replaced
    }
}

impl Default for UndefinedPatternNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

fn is_empty_array(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::ArrayExpression(a) if a.elements.is_empty())
}

impl<'a> Traverse<'a, DeobfuscateState> for UndefinedPatternNormalizer {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        // [][[]]  →  ComputedMemberExpression { object: [], expression: [] }
        if let Expression::ComputedMemberExpression(cme) = expr
            && is_empty_array(&cme.object)
            && is_empty_array(&cme.expression)
        {
            if self.replaced < 10 {
                eprintln!("[AKAMAI/undef-pat] [][[]] -> undefined");
            }
            self.replaced += 1;
            *expr = Expression::new_identifier_reference(SPAN, "undefined", ctx);
        }
    }
}
