//! Phase 17: the statement-split normal form.
//!
//! Four statement-list rewrites expand minified statements into readable ones:
//!
//! ```js
//! a(), b;                 // a(); b;                        (sequence_expression_split)
//! return a(), b;          // a(); return b;                 (return_sequence_split)
//! x && y();               // if (x) { y(); }                (short_circuit_to_if)
//! c ? d() : e();          // if (c) { d(); } else { e(); }  (ternary_to_if_else)
//! ```
//!
//! and a single-statement `if`/`else` or loop body that one of them would
//! expand is wrapped in a block first (`if (x) a(), b;` → `if (x) { a(); b; }`).
//!
//! Each rewrite replaces one statement by statements built from its own
//! subexpressions, and no two rewrites match the same statement, so the
//! rewrites commute and every order of applying them ends in the same normal
//! form. Instead of re-running every pass over the whole program until nothing
//! changes, [`StatementSplitNormalizer`] reaches that form in one traversal:
//! when it rewrites a statement it re-examines the statements the rewrite
//! produced and normalizes the blocks it built, and everything else was
//! already normalized when the traversal left it.
//!
//! Which rewrites apply follows the list's container exactly as the separate
//! passes did: a return-sequence split never applies to `Program` statements,
//! and a ternary is never converted directly in a block-bodied arrow's body.

use oxc_allocator::{ArenaVec, TakeIn};
use oxc_ast::ast::{
    BlockStatement, DoWhileStatement, ForInStatement, ForOfStatement, ForStatement, FunctionBody, IfStatement, Program,
    Statement, SwitchCase, WhileStatement,
};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

use crate::ast_deobfuscate::return_sequence_split::{is_return_sequence, split_return_sequence};
use crate::ast_deobfuscate::sequence_expression_split::{is_sequence_stmt, split_sequence_stmt};
use crate::ast_deobfuscate::short_circuit_to_if::{is_short_circuit_expr_stmt, short_circuit_to_if};
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::ternary_to_if_else::{is_convertible, ternary_to_if};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// The rewrites that depend on the statement list's container; the sequence
/// split and the short-circuit conversion apply in every list.
#[derive(Clone, Copy)]
struct ListRules {
    ternary: bool,
    return_sequence: bool,
}

impl ListRules {
    const ALL: Self = Self {
        ternary: true,
        return_sequence: true,
    };

    fn matches(self, stmt: &Statement<'_>) -> bool {
        is_sequence_stmt(stmt)
            || is_short_circuit_expr_stmt(stmt)
            || (self.ternary && is_convertible(stmt))
            || (self.return_sequence && is_return_sequence(stmt))
    }
}

#[derive(Default)]
pub struct StatementSplitNormalizer {
    rewrite_count: usize,
}

impl StatementSplitNormalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self { rewrite_count: 0 }
    }

    /// Statements rewritten and bodies wrapped in a block.
    #[must_use]
    pub const fn rewrite_count(&self) -> usize {
        self.rewrite_count
    }

    fn normalize_list<'a>(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, rules: ListRules, ctx: &mut Ctx<'a>) {
        if !stmts.iter().any(|stmt| rules.matches(stmt)) {
            return;
        }
        for stmt in stmts.take_in(ctx) {
            self.push_normalized(stmt, rules, stmts, ctx);
        }
    }

    /// Pushes `stmt` to `out` in normal form. Statements a rewrite produces
    /// stay in the same list and are examined again; an `if` a rewrite builds
    /// is new and only its blocks need normalizing.
    fn push_normalized<'a>(
        &mut self,
        stmt: Statement<'a>,
        rules: ListRules,
        out: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut Ctx<'a>,
    ) {
        let stmt = match split_sequence_stmt(stmt, &ctx.ast) {
            Ok(parts) => return self.push_rewritten(parts, rules, out, ctx),
            Err(stmt) => stmt,
        };
        let stmt = match short_circuit_to_if(stmt, ctx) {
            Ok(if_stmt) => return self.push_rewritten(ArenaVec::from_value_in(if_stmt, &ctx.ast), rules, out, ctx),
            Err(stmt) => stmt,
        };
        let stmt = if rules.ternary {
            match ternary_to_if(stmt, ctx) {
                Ok(parts) => return self.push_rewritten(parts, rules, out, ctx),
                Err(stmt) => stmt,
            }
        } else {
            stmt
        };
        let stmt = if rules.return_sequence {
            match split_return_sequence(stmt, &ctx.ast) {
                Ok(parts) => return self.push_rewritten(parts, rules, out, ctx),
                Err(stmt) => stmt,
            }
        } else {
            stmt
        };
        out.push(stmt);
    }

    fn push_rewritten<'a>(
        &mut self,
        parts: ArenaVec<'a, Statement<'a>>,
        rules: ListRules,
        out: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut Ctx<'a>,
    ) {
        self.rewrite_count += 1;
        for mut part in parts {
            if let Statement::IfStatement(if_stmt) = &mut part {
                self.normalize_branches(if_stmt, ctx);
                out.push(part);
            } else {
                self.push_normalized(part, rules, out, ctx);
            }
        }
    }

    fn normalize_branches<'a>(&mut self, if_stmt: &mut IfStatement<'a>, ctx: &mut Ctx<'a>) {
        let IfStatement {
            consequent, alternate, ..
        } = if_stmt;
        for branch in std::iter::once(consequent).chain(alternate.as_mut()) {
            if let Statement::BlockStatement(block) = branch {
                self.normalize_list(&mut block.body, ListRules::ALL, ctx);
            }
        }
    }

    /// Wraps a body that a rewrite would expand into several statements or
    /// an `if`, then normalizes the new block.
    fn normalize_body<'a>(&mut self, body: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if !ListRules::ALL.matches(body) {
            return;
        }
        let mut stmts = ArenaVec::new_in(&ctx.ast);
        stmts.push(body.take_in(ctx));
        let scope_id = ctx.create_child_scope_of_current(ScopeFlags::empty());
        *body = Statement::new_block_statement_with_scope_id(SPAN, stmts, scope_id, &ctx.ast);
        self.rewrite_count += 1;
        let Statement::BlockStatement(block) = body else {
            unreachable!("just built")
        };
        self.normalize_list(&mut block.body, ListRules::ALL, ctx);
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for StatementSplitNormalizer {
    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let rules = ListRules {
            ternary: true,
            return_sequence: false,
        };
        self.normalize_list(&mut program.body, rules, ctx);
    }

    fn exit_block_statement(&mut self, block: &mut BlockStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_list(&mut block.body, ListRules::ALL, ctx);
    }

    fn exit_function_body(&mut self, body: &mut FunctionBody<'a>, ctx: &mut Ctx<'a>) {
        let rules = ListRules {
            // An arrow's body keeps its ternary statements (`ternary_to_if_else`
            // has always skipped them).
            ternary: !matches!(ctx.parent(), Ancestor::ArrowFunctionExpressionBody(_)),
            return_sequence: true,
        };
        self.normalize_list(&mut body.statements, rules, ctx);
    }

    fn exit_switch_case(&mut self, case: &mut SwitchCase<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_list(&mut case.consequent, ListRules::ALL, ctx);
    }

    fn exit_if_statement(&mut self, stmt: &mut IfStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.consequent, ctx);
        if let Some(alternate) = &mut stmt.alternate {
            self.normalize_body(alternate, ctx);
        }
    }

    fn exit_while_statement(&mut self, stmt: &mut WhileStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.body, ctx);
    }

    fn exit_do_while_statement(&mut self, stmt: &mut DoWhileStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.body, ctx);
    }

    fn exit_for_statement(&mut self, stmt: &mut ForStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.body, ctx);
    }

    fn exit_for_in_statement(&mut self, stmt: &mut ForInStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.body, ctx);
    }

    fn exit_for_of_statement(&mut self, stmt: &mut ForOfStatement<'a>, ctx: &mut Ctx<'a>) {
        self.normalize_body(&mut stmt.body, ctx);
    }
}

/// Runs the normalizer alone; returns the code and the rewrite count.
#[cfg(test)]
pub fn normalize(code: &str, source_type: oxc_span::SourceType) -> (String, usize) {
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
    assert!(ret.diagnostics.is_empty(), "test input must parse: {code}");
    let mut program = ret.program;
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    let mut normalizer = StatementSplitNormalizer::new();
    traverse_mut_with_ctx(&mut normalizer, &mut program, &mut ctx);
    (Codegen::new().build(&program).code, normalizer.rewrite_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_span::SourceType;

    fn run(code: &str) -> (String, usize) {
        normalize(code, SourceType::script())
    }

    #[test]
    fn wraps_and_splits_if_and_else_bodies() {
        let (out, count) = run("if (x) a = 1, b(); else c && d();");
        assert_eq!(count, 4, "got: {out}");
        assert_eq!(
            out,
            "if (x) {\n\ta = 1;\n\tb();\n} else {\n\tif (c) {\n\t\td();\n\t}\n}\n"
        );
    }

    #[test]
    fn wraps_loop_bodies() {
        let (out, count) = run(
            "while (x) a(), b(); do c ? d() : e(); while (y); for (;;) f || g(); for (k in o) h(), i(); for (v of l) j && m();",
        );
        assert_eq!(count, 10, "got: {out}");
    }

    #[test]
    fn wraps_return_sequence_body() {
        let (out, count) = run("function f() { if (x) return a(), b; }");
        assert_eq!(count, 2);
        assert!(out.contains("if (x) {\n\t\ta();\n\t\treturn b;\n\t}"), "got: {out}");
    }

    #[test]
    fn leaves_bodies_no_rewrite_expands() {
        let (out, count) = run(
            "function f() { if (x) foo(); else if (y) bar(); while (z) var a = 1, b = 2; for (;;) return c ? d : e; }",
        );
        assert_eq!(count, 0, "got: {out}");
    }

    #[test]
    fn keeps_the_container_rules_of_the_separate_passes() {
        // No return-sequence split at the top level; no ternary conversion
        // directly in an arrow's body (block-bodied or not).
        let (out, count) = normalize(
            "return a(), b;\nx = () => { c ? a() : (b, d); };\ny = () => a && b();\nw = () => c ? a() : b();\n",
            SourceType::cjs(),
        );
        assert_eq!(count, 0, "got: {out}");
    }

    /// The normal form the old whole-program fixpoint (five passes per round,
    /// up to 8 rounds) reached on inputs whose rewrites only become possible
    /// after another rewrite, with the rewrite totals it reported. Captured
    /// from the round-based implementation (e9a6d51).
    #[test]
    fn reaches_the_fixpoint_of_the_round_based_passes() {
        for (input, expected, total) in FIXPOINT_CASES {
            let (out, count) = run(input);
            assert_eq!(out, *expected, "input: {input}");
            assert_eq!(count, *total, "input: {input}");
            let (again, more) = run(&out);
            assert_eq!((again.as_str(), more), (out.as_str(), 0), "not a fixpoint: {input}");
        }
    }

    /// Nesting that needed more rounds than the old 8-round cap is now
    /// normalized completely.
    #[test]
    fn normalizes_nesting_deeper_than_the_old_round_cap() {
        let mut code = "z()".to_string();
        for i in 0..9 {
            code = format!("c{i} ? (s{i} && ({code})) : y{i}()");
        }
        let (out, count) = run(&format!("{code};"));
        assert_eq!(count, 18, "got: {out}");
        assert!(!out.contains("&&") && !out.contains('?'), "got: {out}");
    }

    const FIXPOINT_CASES: &[(&str, &str, usize)] = &[
        (
            "a && (b ? (c, d && (e ? f() : (g, h()))) : (i, j || k()));",
            "if (a) {\n\tif (b) {\n\t\tc;\n\t\tif (d) {\n\t\t\tif (e) {\n\t\t\t\tf();\n\t\t\t} else {\n\t\t\t\tg;\n\t\t\t\th();\n\t\t\t}\n\t\t}\n\t} else {\n\t\ti;\n\t\tif (!j) {\n\t\t\tk();\n\t\t}\n\t}\n}\n",
            8,
        ),
        (
            "function f() { return a ? (b, c) : d ? (e, f && g) : (h, i); }",
            "function f() {\n\tif (a) {\n\t\tb;\n\t\treturn c;\n\t}\n\tif (d) {\n\t\te;\n\t\treturn f && g;\n\t}\n\th;\n\treturn i;\n}\n",
            5,
        ),
        (
            "if (x) a ? (b(), c && d()) : e || (f(), g ? h() : i()); else for (;;) j && (k, l);",
            "if (x) {\n\tif (a) {\n\t\tb();\n\t\tif (c) {\n\t\t\td();\n\t\t}\n\t} else {\n\t\tif (!e) {\n\t\t\tf();\n\t\t\tif (g) {\n\t\t\t\th();\n\t\t\t} else {\n\t\t\t\ti();\n\t\t\t}\n\t\t}\n\t}\n} else for (;;) {\n\tif (j) {\n\t\tk;\n\t\tl;\n\t}\n}\n",
            10,
        ),
        (
            "a && (b, c && (d, e ? (f, g) : h));",
            "if (a) {\n\tb;\n\tif (c) {\n\t\td;\n\t\tif (e) {\n\t\t\tf;\n\t\t\tg;\n\t\t} else {\n\t\t\th;\n\t\t}\n\t}\n}\n",
            6,
        ),
        (
            "function f(k) { switch (k) { case 1: a && (b, c); default: return x ? (y, z) : w; } }",
            "function f(k) {\n\tswitch (k) {\n\t\tcase 1: if (a) {\n\t\t\tb;\n\t\t\tc;\n\t\t}\n\t\tdefault:\n\t\t\tif (x) {\n\t\t\t\ty;\n\t\t\t\treturn z;\n\t\t\t}\n\t\t\treturn w;\n\t}\n}\n",
            4,
        ),
        (
            "x = () => { c ? a() : (b, d); }; y = () => a && b(); z = () => (a, b); w = () => c ? a() : b();",
            "x = () => {\n\tc ? a() : (b, d);\n};\ny = () => a && b();\nz = () => (a, b);\nw = () => c ? a() : b();\n",
            0,
        ),
        (
            "function f() { if (a) return b(), c ? (d, e) : f; while (x) y || (z, w ? (v(), u) : t); }",
            "function f() {\n\tif (a) {\n\t\tb();\n\t\tif (c) {\n\t\t\td;\n\t\t\treturn e;\n\t\t}\n\t\treturn f;\n\t}\n\twhile (x) {\n\t\tif (!y) {\n\t\t\tz;\n\t\t\tif (w) {\n\t\t\t\tv();\n\t\t\t\tu;\n\t\t\t} else {\n\t\t\t\tt;\n\t\t\t}\n\t\t}\n\t}\n}\n",
            9,
        ),
        (
            "do p ? (q, r) : s && t; while (u); for (k in o) k && (m, n); for (v of l) v, w ? 1 : 2;",
            "do {\n\tif (p) {\n\t\tq;\n\t\tr;\n\t} else {\n\t\tif (s) {\n\t\t\tt;\n\t\t}\n\t}\n} while (u);\nfor (k in o) {\n\tif (k) {\n\t\tm;\n\t\tn;\n\t}\n}\nfor (v of l) {\n\tv;\n\tif (w) {\n\t\t1;\n\t} else {\n\t\t2;\n\t}\n}\n",
            10,
        ),
        (
            "function f() { return a(), b ? (c && d, e) : (f, g ? h : (i, j)); }",
            "function f() {\n\ta();\n\tif (b) {\n\t\tif (c) {\n\t\t\td;\n\t\t}\n\t\treturn e;\n\t}\n\tf;\n\tif (g) {\n\t\treturn h;\n\t}\n\ti;\n\treturn j;\n}\n",
            7,
        ),
        (
            "!a || (b = 1, c !== d && (e = 2, f == g || h()));",
            "if (a) {\n\tb = 1;\n\tif (c !== d) {\n\t\te = 2;\n\t\tif (f != g) {\n\t\t\th();\n\t\t}\n\t}\n}\n",
            5,
        ),
        (
            "(a, b) && c(); ((a)) ? (b) : (c, d);",
            "if (a, b) {\n\tc();\n}\nif (a) {\n\tb;\n} else {\n\tc;\n\td;\n}\n",
            3,
        ),
        ("l: a, b; with (o) a && b;", "l: a, b;\nwith(o) a && b;\n", 0),
        (
            "function f() { { a ? b() : c(); } label: { d && e(); } try { f, g } catch (e) { h || i(); } finally { j ? k : l; } }",
            "function f() {\n\t{\n\t\tif (a) {\n\t\t\tb();\n\t\t} else {\n\t\t\tc();\n\t\t}\n\t}\n\tlabel: {\n\t\tif (d) {\n\t\t\te();\n\t\t}\n\t}\n\ttry {\n\t\tf;\n\t\tg;\n\t} catch (e) {\n\t\tif (!h) {\n\t\t\ti();\n\t\t}\n\t} finally {\n\t\tif (j) {\n\t\t\tk;\n\t\t} else {\n\t\t\tl;\n\t\t}\n\t}\n}\n",
            5,
        ),
        (
            "class A { m() { a && (b, c ? d() : e()); } static { f, g && h(); } }",
            "class A {\n\tm() {\n\t\tif (a) {\n\t\t\tb;\n\t\t\tif (c) {\n\t\t\t\td();\n\t\t\t} else {\n\t\t\t\te();\n\t\t\t}\n\t\t}\n\t}\n\tstatic {\n\t\tf, g && h();\n\t}\n}\n",
            3,
        ),
        (
            "c2 ? (s2 && (c1 ? (s1 && (c0 ? (s0 && (z())) : y0())) : y1())) : y2();",
            "if (c2) {\n\tif (s2) {\n\t\tif (c1) {\n\t\t\tif (s1) {\n\t\t\t\tif (c0) {\n\t\t\t\t\tif (s0) {\n\t\t\t\t\t\tz();\n\t\t\t\t\t}\n\t\t\t\t} else {\n\t\t\t\t\ty0();\n\t\t\t\t}\n\t\t\t}\n\t\t} else {\n\t\t\ty1();\n\t\t}\n\t}\n} else {\n\ty2();\n}\n",
            6,
        ),
    ];
}
