//! String-array-factory dispatch inliner.
//!
//! Resolves `x19()[CONST]` and `Y49()[CONST]` patterns into literal strings.
//!
//! Detects functions that return a literal string array:
//!
//! ```text
//! function x19() { return ["BR","t5","B5",...]; }
//! ```
//!
//! or the self-init accessor pattern:
//!
//! ```text
//! function x19() {
//!     var vB4 = ["BR","t5","B5",...];
//!     x19 = function() { return vB4; };
//!     return vB4;
//! }
//! ```
//!
//! Then replaces `x19()[5]` with `"BE"` (the string at index 5).
//! Also resolves simple `var CONST = NUMBER;` index constants.

use oxc_ast::ast::{
    AssignmentTarget, BinaryOperator, BindingPattern, Expression, Function, Statement, StringLiteral, UnaryOperator,
    VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

fn try_eval_expr(expr: &Expression<'_>, known: &FxHashMap<String, usize>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) => {
            let v = n.value;
            if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                return None;
            }
            Some(v as i64)
        }
        Expression::Identifier(id) => known.get(id.name.as_str()).map(|&v| v as i64),
        Expression::BinaryExpression(bin) => {
            let l = try_eval_expr(&bin.left, known)?;
            let r = try_eval_expr(&bin.right, known)?;
            match bin.operator {
                BinaryOperator::Addition => l.checked_add(r),
                BinaryOperator::Subtraction => l.checked_sub(r),
                BinaryOperator::Multiplication => l.checked_mul(r),
                _ => None,
            }
        }
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => try_eval_expr(&u.argument, known),
        Expression::ParenthesizedExpression(p) => try_eval_expr(&p.expression, known),
        _ => None,
    }
}

fn try_eval_as_usize(expr: &Expression<'_>, known: &FxHashMap<String, usize>) -> Option<usize> {
    let v = try_eval_expr(expr, known)?;
    usize::try_from(v).ok()
}

enum ConstExpr {
    Lit(i64),
    Ident(String),
    Add(Box<ConstExpr>, Box<ConstExpr>),
    Sub(Box<ConstExpr>, Box<ConstExpr>),
    Mul(Box<ConstExpr>, Box<ConstExpr>),
}

impl ConstExpr {
    fn from_ast(expr: &Expression<'_>) -> Option<Self> {
        match expr {
            Expression::NumericLiteral(n) => {
                let v = n.value;
                if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                    return None;
                }
                Some(Self::Lit(v as i64))
            }
            Expression::Identifier(id) => Some(Self::Ident(id.name.as_str().to_string())),
            Expression::BinaryExpression(bin) => {
                let l = Box::new(Self::from_ast(&bin.left)?);
                let r = Box::new(Self::from_ast(&bin.right)?);
                match bin.operator {
                    BinaryOperator::Addition => Some(Self::Add(l, r)),
                    BinaryOperator::Subtraction => Some(Self::Sub(l, r)),
                    BinaryOperator::Multiplication => Some(Self::Mul(l, r)),
                    _ => None,
                }
            }
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryPlus => Self::from_ast(&u.argument),
            Expression::ParenthesizedExpression(p) => Self::from_ast(&p.expression),
            _ => None,
        }
    }

    fn eval(&self, known: &FxHashMap<String, usize>) -> Option<i64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Ident(name) => known.get(name.as_str()).map(|&v| v as i64),
            Self::Add(l, r) => l.eval(known)?.checked_add(r.eval(known)?),
            Self::Sub(l, r) => l.eval(known)?.checked_sub(r.eval(known)?),
            Self::Mul(l, r) => l.eval(known)?.checked_mul(r.eval(known)?),
        }
    }
}

pub struct DispatchInlinerCollector {
    factories: FxHashMap<String, Vec<String>>,
    constants: FxHashMap<String, usize>,
    unresolved: Vec<(String, ConstExpr)>,
}

impl DispatchInlinerCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            factories: FxHashMap::default(),
            constants: FxHashMap::default(),
            unresolved: Vec::new(),
        }
    }

    #[must_use]
    pub fn into_maps(mut self) -> (FxHashMap<String, Vec<String>>, FxHashMap<String, usize>) {
        // Multi-pass: resolve constants that depend on other constants
        for _ in 0..10 {
            let prev_len = self.constants.len();
            self.unresolved.retain(|(name, expr)| {
                if let Some(v) = expr.eval(&self.constants) {
                    if let Ok(u) = usize::try_from(v) {
                        self.constants.insert(name.clone(), u);
                        return false;
                    }
                }
                true
            });
            if self.constants.len() == prev_len {
                break;
            }
        }
        (self.factories, self.constants)
    }

    fn extract_string_array(expr: &Expression<'_>) -> Option<Vec<String>> {
        let Expression::ArrayExpression(arr) = expr else {
            return None;
        };
        let mut strings = Vec::with_capacity(arr.elements.len());
        for elem in &arr.elements {
            let expr = elem.as_expression()?;
            let Expression::StringLiteral(s) = expr else {
                return None;
            };
            strings.push(s.value.as_str().to_string());
        }
        Some(strings)
    }

    fn try_direct_return(func: &Function<'_>) -> Option<Vec<String>> {
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };
        let arg = ret.argument.as_ref()?;
        Self::extract_string_array(arg)
    }

    fn try_self_init_accessor(func: &Function<'_>) -> Option<Vec<String>> {
        let func_name = func.id.as_ref()?.name.as_str();
        let body = func.body.as_ref()?;
        if body.statements.len() != 3 {
            return None;
        }

        let Statement::VariableDeclaration(var_decl) = &body.statements[0] else {
            return None;
        };
        if var_decl.declarations.len() != 1 {
            return None;
        }
        let decl = &var_decl.declarations[0];
        let BindingPattern::BindingIdentifier(local_id) = &decl.id else {
            return None;
        };
        let local_name = local_id.name.as_str();
        let init = decl.init.as_ref()?;
        let strings = Self::extract_string_array(init)?;

        let Statement::ExpressionStatement(expr_stmt) = &body.statements[1] else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
            return None;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target_id) = &assign.left else {
            return None;
        };
        if target_id.name.as_str() != func_name {
            return None;
        }
        let Expression::FunctionExpression(inner_fn) = &assign.right else {
            return None;
        };
        let inner_body = inner_fn.body.as_ref()?;
        if inner_body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(inner_ret) = &inner_body.statements[0] else {
            return None;
        };
        let Some(Expression::Identifier(ret_id)) = &inner_ret.argument else {
            return None;
        };
        if ret_id.name.as_str() != local_name {
            return None;
        }

        let Statement::ReturnStatement(ret) = &body.statements[2] else {
            return None;
        };
        let Some(Expression::Identifier(ret_id2)) = &ret.argument else {
            return None;
        };
        if ret_id2.name.as_str() != local_name {
            return None;
        }

        Some(strings)
    }

    fn try_extract_factory(func: &Function<'_>) -> Option<Vec<String>> {
        Self::try_direct_return(func).or_else(|| Self::try_self_init_accessor(func))
    }
}

impl Default for DispatchInlinerCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id
            && let Some(strings) = Self::try_extract_factory(func)
        {
            let name = id.name.as_str().to_string();
            eprintln!(
                "[AST/dispatch-inline] found string-array factory {}() with {} elements",
                name,
                strings.len()
            );
            self.factories.insert(name, strings);
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let Some(init) = &decl.init else {
            return;
        };
        let name = id.name.as_str().to_string();
        if let Some(val) = try_eval_as_usize(init, &self.constants) {
            self.constants.insert(name, val);
        } else if let Some(ce) = ConstExpr::from_ast(init) {
            self.unresolved.push((name, ce));
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        let Statement::ExpressionStatement(es) = stmt else {
            return;
        };
        let Expression::AssignmentExpression(assign) = &es.expression else {
            return;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return;
        };
        let name = target.name.as_str().to_string();
        if let Some(val) = try_eval_as_usize(&assign.right, &self.constants) {
            self.constants.insert(name, val);
        } else if let Some(ce) = ConstExpr::from_ast(&assign.right) {
            self.unresolved.push((name, ce));
        }
    }
}

pub struct DispatchInlinerRewriter {
    factories: FxHashMap<String, Vec<String>>,
    constants: FxHashMap<String, usize>,
    inlined: usize,
}

impl DispatchInlinerRewriter {
    #[must_use]
    pub fn new(factories: FxHashMap<String, Vec<String>>, constants: FxHashMap<String, usize>) -> Self {
        Self {
            factories,
            constants,
            inlined: 0,
        }
    }

    #[must_use]
    pub const fn inlined(&self) -> usize {
        self.inlined
    }

    fn resolve_index(&self, expr: &Expression<'_>) -> Option<usize> {
        try_eval_as_usize(expr, &self.constants)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerRewriter {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::ComputedMemberExpression(cme) = expr else {
            return;
        };
        let Expression::CallExpression(call) = &cme.object else {
            return;
        };
        if !call.arguments.is_empty() {
            return;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let Some(strings) = self.factories.get(callee.name.as_str()) else {
            return;
        };
        let Some(index) = self.resolve_index(&cme.expression) else {
            return;
        };
        let Some(value) = strings.get(index) else {
            return;
        };

        if self.inlined < 10 {
            eprintln!(
                "[AST/dispatch-inline] {}()[{}] -> {:?}",
                callee.name.as_str(),
                index,
                value
            );
        }
        self.inlined += 1;

        *expr = Expression::StringLiteral(ctx.ast.alloc(StringLiteral {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            value: ctx.ast.str(value),
            raw: None,
            lone_surrogates: false,
        }));
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

        let mut collector = DispatchInlinerCollector::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

        let (factories, constants) = collector.into_maps();
        if !factories.is_empty() {
            let mut rewriter = DispatchInlinerRewriter::new(factories, constants);
            let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
            let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
            traverse_mut_with_ctx(&mut rewriter, &mut program, &mut ctx);
        }

        Codegen::new().build(&program).code
    }

    #[test]
    fn resolves_simple_array_factory() {
        let out = run(r#"function f(){return ["a","b","c"];} var r = f()[1];"#);
        assert!(out.contains("\"b\""), "got: {out}");
        assert!(!out.contains("f()[1]"), "call site should be replaced: {out}");
    }

    #[test]
    fn resolves_self_init_accessor_array() {
        let code = r#"
            function f() {
                var a = ["x", "y"];
                f = function() { return a; };
                return a;
            }
            var r = f()[0];
        "#;
        let out = run(code);
        assert!(out.contains("\"x\""), "got: {out}");
        assert!(!out.contains("f()[0]"), "call should be replaced: {out}");
    }

    #[test]
    fn resolves_with_index_constant() {
        let code = r#"
            var I = 2;
            function f(){return ["a","b","c"];}
            var r = f()[I];
        "#;
        let out = run(code);
        assert!(out.contains("\"c\""), "got: {out}");
    }

    #[test]
    fn preserves_non_constant_index() {
        let code = r#"function f(){return ["a","b"];} var r = f()[x];"#;
        let out = run(code);
        assert!(out.contains("f()[x]"), "non-constant index must be preserved: {out}");
    }

    #[test]
    fn preserves_non_array_factory() {
        let code = r#"function f(){return compute();} var r = f()[0];"#;
        let out = run(code);
        assert!(out.contains("f()[0]"), "non-array factory must be preserved: {out}");
    }

    #[test]
    fn handles_large_array() {
        let mut elements: Vec<String> = Vec::new();
        for i in 0..150 {
            elements.push(format!("\"s{}\"", i));
        }
        let array_str = elements.join(",");
        let code = format!("function f(){{return [{array_str}];}} var r = f()[149];");
        let out = run(&code);
        assert!(out.contains("\"s149\""), "got: {out}");
    }

    #[test]
    fn resolves_multiple_factories() {
        let code = r#"
            function x19(){return ["BR","t5","B5"];}
            function Y49(){return ["HR","tg","gX"];}
            var a = x19()[0];
            var b = Y49()[2];
        "#;
        let out = run(code);
        assert!(out.contains("\"BR\""), "x19()[0] should resolve: {out}");
        assert!(out.contains("\"gX\""), "Y49()[2] should resolve: {out}");
    }

    #[test]
    fn out_of_bounds_index_preserved() {
        let code = r#"function f(){return ["a","b"];} var r = f()[5];"#;
        let out = run(code);
        assert!(out.contains("f()[5]"), "out-of-bounds must be preserved: {out}");
    }

    #[test]
    fn ignores_factory_with_args() {
        let code = r#"function f(){return ["a","b"];} var r = f(1)[0];"#;
        let out = run(code);
        assert!(out.contains("f(1)"), "call with args must be preserved: {out}");
    }

    #[test]
    fn resolves_bare_assignment_constant() {
        let code = r#"NF = 0; function f(){return ["a","b"];} var r = f()[NF];"#;
        let out = run(code);
        assert!(out.contains("\"a\""), "bare assignment constant should resolve: {out}");
        assert!(!out.contains("f()[NF]"), "call site should be replaced: {out}");
    }

    #[test]
    fn resolves_mixed_var_and_bare() {
        let code = r#"var I = 1; J = 2; function f(){return ["a","b","c"];} var r = f()[J];"#;
        let out = run(code);
        assert!(out.contains("\"c\""), "bare assignment constant should resolve: {out}");
        assert!(!out.contains("f()[J]"), "call site should be replaced: {out}");
    }

    #[test]
    fn resolves_computed_constant() {
        let code = r#"var A = 1; var B = A + A; function f(){return ["x","y","z"];} var r = f()[B];"#;
        let out = run(code);
        assert!(
            out.contains("\"z\""),
            "computed constant B=2 should resolve to index 2: {out}"
        );
        assert!(!out.contains("f()[B]"), "call site should be replaced: {out}");
    }

    #[test]
    fn resolves_chain() {
        let code = r#"X = 2; Y = X * 3; Z = Y - 1; function f(){return ["a","b","c","d","e","f"];} var r = f()[Z];"#;
        let out = run(code);
        assert!(
            out.contains("\"f\""),
            "chained constant Z=5 should resolve to index 5: {out}"
        );
        assert!(!out.contains("f()[Z]"), "call site should be replaced: {out}");
    }
}
