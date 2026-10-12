//! String `.concat` canonicaliser.
//!
//! Obfuscators and TypeScript targeting ES5 rewrite template literals as
//! `.concat` chains. This pass turns them back into template literals:
//!
//! ```text
//! "".concat(a, "=").concat(b)   ->  `${a}=${b}`
//! "prefix".concat(x)            ->  `prefix${x}`
//! a.concat(b)                   ->  (left alone: `a` is not provably a string)
//! ```
//!
//! Both forms convert each value with `ToString`, unlike `+`, which uses
//! `ToPrimitive` with the `default` hint. They differ in order: `.concat`
//! evaluates every argument before converting any, while a template converts
//! each substitution right after evaluating it. The rewrite therefore only
//! fires when at most one argument of a call is not a primitive literal, so no
//! conversion can observe or affect the evaluation of a later argument.

use oxc_allocator::{CloneIn, GetAllocator, Vec as ArenaVec};
use oxc_ast::ast::{Argument, Expression, TemplateElement};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::dyn_concat_resolver::literal::make_template_element;
use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ConcatCanonicaliser {
    rewrites: usize,
}

counter_pass!(ConcatCanonicaliser, rewrites, rewrites);

const fn is_primitive_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
    )
}

/// Template pieces: `quasis.len() == expressions.len() + 1`.
struct Pieces<'a> {
    quasis: Vec<String>,
    expressions: Vec<Expression<'a>>,
}

impl<'a> Pieces<'a> {
    fn from_receiver(receiver: &Expression<'a>, ctx: &Ctx<'a>) -> Option<Self> {
        match receiver {
            Expression::StringLiteral(lit) if !lit.lone_surrogates => Some(Self {
                quasis: vec![lit.value.to_string()],
                expressions: Vec::new(),
            }),
            Expression::TemplateLiteral(tmpl) => {
                let quasis = tmpl
                    .quasis
                    .iter()
                    .map(|quasi| {
                        quasi
                            .value
                            .cooked
                            .filter(|_| !quasi.lone_surrogates)
                            .map(|c| c.to_string())
                    })
                    .collect::<Option<Vec<_>>>()?;
                let expressions = tmpl
                    .expressions
                    .iter()
                    .map(|expr| expr.clone_in_with_semantic_ids(ctx.allocator()))
                    .collect();
                Some(Self { quasis, expressions })
            }
            _ => None,
        }
    }

    fn push(&mut self, arg: &Expression<'a>, ctx: &Ctx<'a>) {
        if let Expression::StringLiteral(lit) = arg {
            self.quasis
                .last_mut()
                .expect("pieces always hold a quasi")
                .push_str(&lit.value);
        } else {
            self.expressions.push(arg.clone_in_with_semantic_ids(ctx.allocator()));
            self.quasis.push(String::new());
        }
    }

    fn into_expression(self, ctx: &Ctx<'a>) -> Expression<'a> {
        if self.expressions.is_empty() {
            return Expression::new_string_literal(SPAN, ctx.allocator().alloc_str(&self.quasis[0]), None, ctx);
        }
        let last = self.quasis.len() - 1;
        let mut quasis: ArenaVec<'a, TemplateElement<'a>> = ArenaVec::new_in(ctx);
        for (i, quasi) in self.quasis.iter().enumerate() {
            quasis.push(make_template_element(quasi, i == last, ctx));
        }
        let mut expressions = ArenaVec::new_in(ctx);
        expressions.extend(self.expressions);
        Expression::new_template_literal(SPAN, quasis, expressions, ctx)
    }
}

fn concat_arguments<'b, 'a>(arguments: &'b [Argument<'a>]) -> Option<Vec<&'b Expression<'a>>> {
    let args = arguments
        .iter()
        .map(|arg| arg.as_expression())
        .collect::<Option<Vec<_>>>()?;
    let lone_surrogate = args
        .iter()
        .any(|arg| matches!(arg, Expression::StringLiteral(lit) if lit.lone_surrogates));
    let non_literals = args.iter().filter(|arg| !is_primitive_literal(arg)).count();
    (!lone_surrogate && non_literals <= 1).then_some(args)
}

impl<'a> Traverse<'a, DeobfuscateState> for ConcatCanonicaliser {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return;
        };
        if sme.property.name.as_str() != "concat" {
            return;
        }
        let Some(args) = concat_arguments(&call.arguments) else {
            return;
        };
        let Some(mut pieces) = Pieces::from_receiver(&sme.object, ctx) else {
            return;
        };
        for arg in args {
            pieces.push(arg, ctx);
        }
        if self.rewrites < 10 {
            deob_trace!(
                "[AST/concat] rewriting <str>.concat({} args) -> template literal",
                call.arguments.len()
            );
        }
        self.rewrites += 1;
        *expr = pieces.into_expression(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = ConcatCanonicaliser::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn typescript_concat_chain() {
        assert_eq!(run("x = \"\".concat(a, \"=\").concat(b);"), "x = `${a}=${b}`;\n");
    }

    #[test]
    fn prefix_concat() {
        assert_eq!(run("x = \"prefix\".concat(y);"), "x = `prefix${y}`;\n");
    }

    #[test]
    fn literal_only_concat() {
        assert_eq!(run("x = \"a\".concat(\"`${b}\\\\\");"), "x = \"a`${b}\\\\\";\n");
    }

    #[test]
    fn escapes_template_syntax_in_quasis() {
        assert_eq!(run("x = \"`\".concat(y, \"${z}\\\\\");"), "x = `\\`${y}\\${z}\\\\`;\n");
    }

    #[test]
    fn leaves_non_string_receiver_alone() {
        let code = "x = arr.concat(y, z);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_two_non_literal_arguments_alone() {
        let code = "x = \"\".concat(a, b, c, d, e);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_spread_alone() {
        let code = "x = \"\".concat(...a);\n";
        assert_eq!(run(code), code);
    }
}
