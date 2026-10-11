//! `let` -> `const` for bindings that are initialized and never written.
//!
//! Minifiers print `const` as `let` (it is shorter and behaves the same once
//! the program is known not to write the binding). Restoring `const` signals
//! intent. The two kinds share TDZ and block scoping; the only observable
//! difference is a write, so a declaration is converted only when no resolved
//! reference to any of its bindings writes, and no direct `eval` (which could
//! write by name) can see them. Top-level bindings are skipped: in a classic
//! script they live in the global lexical environment that other scripts can
//! assign.

use oxc_ast::ast::{VariableDeclaration, VariableDeclarationKind};
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct LetToConst {
    converted_count: usize,
}

counter_pass!(LetToConst, converted_count, converted_count);

impl<'a> Traverse<'a, DeobfuscateState> for LetToConst {
    fn exit_variable_declaration(&mut self, decl: &mut VariableDeclaration<'a>, ctx: &mut Ctx<'a>) {
        if decl.kind != VariableDeclarationKind::Let || decl.declare {
            return;
        }
        let loop_head = matches!(
            ctx.parent(),
            Ancestor::ForInStatementLeft(_) | Ancestor::ForOfStatementLeft(_)
        );
        if !loop_head && decl.declarations.iter().any(|declarator| declarator.init.is_none()) {
            return;
        }
        let scoping = ctx.scoping();
        let never_written = decl.declarations.iter().all(|declarator| {
            declarator.id.get_binding_identifiers().iter().all(|binding| {
                binding.symbol_id.get().is_some_and(|symbol| {
                    let scope = scoping.symbol_scope_id(symbol);
                    scope != scoping.root_scope_id()
                        && !scoping.scope_flags(scope).contains_direct_eval()
                        && !scoping.symbol_is_mutated(symbol)
                })
            })
        });
        if never_written {
            decl.kind = VariableDeclarationKind::Const;
            self.converted_count += 1;
        }
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

    fn converted(code: &str, source_type: SourceType) -> usize {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(
            ret.diagnostics.is_empty(),
            "test input must parse: {:?}",
            ret.diagnostics
        );
        let mut program = ret.program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        let mut pass = LetToConst::new();
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        let out = Codegen::new().build(&program).code;
        let reparsed = Parser::new(&allocator, &out, source_type).parse();
        assert!(reparsed.diagnostics.is_empty(), "output must parse: {out}");
        pass.converted_count()
    }

    fn in_fn(body: &str) -> usize {
        converted(&format!("function f(o, s, xs) {{ {body} }}"), SourceType::mjs())
    }

    #[test]
    fn converts_initialized_unwritten_bindings() {
        assert_eq!(in_fn("let a = 1; return a;"), 1);
        assert_eq!(in_fn("let { a, b: [c] } = o; return a + c;"), 1);
        assert_eq!(in_fn("let a = 1; return () => a;"), 1);
        assert_eq!(in_fn("for (let i = 0; i < 3;) return i;"), 1);
    }

    #[test]
    fn converts_unwritten_for_in_of_heads() {
        assert_eq!(in_fn("for (let k of xs) use(k); for (let k in o) use(k);"), 2);
    }

    #[test]
    fn keeps_let_for_any_write() {
        for body in [
            "let a = 1; a = 2;",
            "let a = 1; a++;",
            "let a = 1; a += 1;",
            "let a = 1; a ||= 1;",
            "let a = 1; [a] = xs;",
            "let a = 1; ({ a } = o);",
            "let a = 1; for (a of xs);",
            "let a = 1; return () => { a = 2; };",
            "for (let i = 0; i < 3; i++);",
            "for (let k of xs) k = 1;",
            "let a = 1, b = 2; b = 3;",
        ] {
            assert_eq!(in_fn(body), 0, "{body}");
        }
    }

    #[test]
    fn keeps_uninitialized_let() {
        assert_eq!(in_fn("let a; return a;"), 0);
    }

    #[test]
    fn keeps_let_written_in_its_temporal_dead_zone() {
        assert_eq!(in_fn("a = 0; let a = 1;"), 0);
    }

    #[test]
    fn keeps_let_visible_to_direct_eval() {
        assert_eq!(in_fn("let a = 1; eval(s); return a;"), 0);
        assert_eq!(in_fn("let a = 1; return () => eval(s);"), 0);
        assert_eq!(in_fn("let a = 1; (eval)(s); return a;"), 0);
        // An indirect eval runs in global scope and cannot see `a`.
        assert_eq!(in_fn("let a = 1; (0, eval)(s); return a;"), 1);
    }

    #[test]
    fn keeps_let_written_through_with() {
        assert_eq!(
            converted(
                "function f(o) { let a = 1; with (o) { a = 2; } return a; }",
                SourceType::script()
            ),
            0
        );
    }

    #[test]
    fn keeps_top_level_let() {
        // Other classic scripts can assign a global lexical binding.
        assert_eq!(converted("let a = 1; use(a);", SourceType::script()), 0);
        assert_eq!(converted("export let a = 1;", SourceType::mjs()), 0);
    }
}
