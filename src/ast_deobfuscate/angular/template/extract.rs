use oxc_ast::AstKind;
use oxc_ast::ast::{
    Argument, ArrowFunctionBody, BindingPattern, CallExpression, Expression, FormalParameters, Function, FunctionBody,
    LogicalOperator, Statement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Semantic;
use oxc_syntax::operator::BinaryOperator;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Identity of an instruction function. Unmangled `ɵɵ*` instructions and
/// unresolved callees are keyed by name, local functions by symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Callee {
    Symbol(SymbolId),
    Name(String),
}

impl Callee {
    pub(super) fn ivy_name(&self) -> Option<&str> {
        match self {
            Self::Name(name) => name.strip_prefix("ɵɵ"),
            Self::Symbol(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Call<'a> {
    pub(super) callee_index: usize,
    pub(super) args: &'a [Argument<'a>],
}

impl<'a> Call<'a> {
    pub(super) fn arg(&self, index: usize) -> Option<&'a Expression<'a>> {
        self.args.get(index).and_then(Argument::as_expression)
    }

    pub(super) fn int(&self, index: usize) -> Option<u32> {
        self.arg(index).and_then(int_literal)
    }

    pub(super) fn string(&self, index: usize) -> Option<&'a str> {
        match self.arg(index)? {
            Expression::StringLiteral(s) => Some(s.value.as_str()),
            _ => None,
        }
    }
}

pub(super) fn int_literal(expr: &Expression<'_>) -> Option<u32> {
    match expr.without_parentheses() {
        Expression::NumericLiteral(n) if n.value >= 0.0 && n.value.fract() == 0.0 && n.value <= f64::from(u32::MAX) => {
            Some(n.value as u32)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) enum Body<'a> {
    Block(&'a FunctionBody<'a>),
    Expression(&'a Expression<'a>),
}

/// A template (or embedded-view) function `function(rf, ctx) { ... }`, or a
/// handler / track function.
#[derive(Clone, Copy)]
pub(super) struct TemplateFn<'a> {
    key: u32,
    params: &'a FormalParameters<'a>,
    pub(super) body: Body<'a>,
}

impl<'a> TemplateFn<'a> {
    pub(super) const fn key(self) -> u32 {
        self.key
    }

    pub(super) fn param(self, index: usize) -> Option<SymbolId> {
        match &self.params.items.get(index)?.pattern {
            BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
            _ => None,
        }
    }

    fn statements(self) -> &'a [Statement<'a>] {
        match self.body {
            Body::Block(body) => &body.statements,
            Body::Expression(_) => &[],
        }
    }

    /// The returned expression of a handler: `function() { ...; return expr }`.
    pub(super) fn returned(self) -> Option<&'a Expression<'a>> {
        match self.body {
            Body::Expression(expr) => Some(expr),
            Body::Block(body) => match body.statements.last()? {
                Statement::ReturnStatement(ret) => ret.argument.as_ref(),
                Statement::ExpressionStatement(es) if body.statements.len() == 1 => Some(&es.expression),
                _ => None,
            },
        }
    }
}

/// Resolve a function expression, arrow, or identifier bound to one.
pub(super) fn as_function<'a>(expr: &'a Expression<'a>, semantic: &Semantic<'a>) -> Option<TemplateFn<'a>> {
    match expr.without_parentheses() {
        Expression::FunctionExpression(f) => function(f),
        Expression::ArrowFunctionExpression(a) => Some(TemplateFn {
            key: a.span.start,
            params: &a.params,
            body: match &a.body {
                ArrowFunctionBody::FunctionBody(body) => Body::Block(body),
                body => Body::Expression(body.as_expression()?),
            },
        }),
        Expression::Identifier(id) => {
            let symbol = semantic.scoping().get_reference(id.reference_id()).symbol_id()?;
            match semantic.symbol_declaration(symbol).kind() {
                AstKind::Function(f) => function(f),
                AstKind::VariableDeclarator(decl) if !semantic.scoping().symbol_is_mutated(symbol) => {
                    as_function(decl.init.as_ref()?, semantic)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn function<'a>(f: &'a Function<'a>) -> Option<TemplateFn<'a>> {
    Some(TemplateFn {
        key: f.span.start,
        params: &f.params,
        body: Body::Block(f.body.as_ref()?),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Create,
    Update,
}

/// Creation- and update-block instruction calls of one template function,
/// in source order. Chained calls (`f(a)(b)`) are flattened.
#[derive(Default)]
pub(super) struct Instructions<'a> {
    pub(super) create: Vec<Call<'a>>,
    pub(super) update: Vec<Call<'a>>,
    /// Slots referenced as `pipeBindN(slot, offset, ...)` in the update block.
    pub(super) pipe_slots: FxHashSet<u32>,
}

pub(super) struct Extractor<'s, 'a> {
    semantic: &'s Semantic<'a>,
    pub(super) callees: Vec<Callee>,
    callee_index: FxHashMap<Callee, usize>,
    flags: Option<SymbolId>,
    out: Instructions<'a>,
}

impl<'s, 'a> Extractor<'s, 'a> {
    pub(super) fn new(semantic: &'s Semantic<'a>) -> Self {
        Self {
            semantic,
            callees: Vec::new(),
            callee_index: FxHashMap::default(),
            flags: None,
            out: Instructions::default(),
        }
    }

    pub(super) fn extract(&mut self, func: TemplateFn<'a>) -> Instructions<'a> {
        self.flags = func.param(0);
        for stmt in func.statements() {
            self.statement(stmt, None);
        }
        let out = std::mem::take(&mut self.out);
        let mut pipes = PipeSlots::default();
        for call in &out.update {
            for arg in call.args {
                if let Some(expr) = arg.as_expression() {
                    pipes.visit_expression(expr);
                }
            }
        }
        Instructions {
            pipe_slots: pipes.0,
            ..out
        }
    }

    fn statement(&mut self, stmt: &'a Statement<'a>, phase: Option<Phase>) {
        match stmt {
            Statement::ExpressionStatement(es) => self.expression(&es.expression, phase),
            Statement::BlockStatement(block) => {
                for stmt in &block.body {
                    self.statement(stmt, phase);
                }
            }
            Statement::IfStatement(if_stmt) => {
                let inner = self.test(&if_stmt.test, phase);
                self.statement(&if_stmt.consequent, inner);
                if let Some(alternate) = &if_stmt.alternate {
                    self.statement(alternate, phase);
                }
            }
            _ => {}
        }
    }

    /// Walks an `if` test and returns the phase its consequent runs in.
    fn test(&mut self, test: &'a Expression<'a>, phase: Option<Phase>) -> Option<Phase> {
        match test.without_parentheses() {
            Expression::SequenceExpression(seq) => {
                let (last, rest) = seq.expressions.split_last()?;
                for expr in rest {
                    self.expression(expr, phase);
                }
                self.test(last, phase)
            }
            expr => self.phase_of(expr).or_else(|| {
                self.expression(expr, phase);
                phase
            }),
        }
    }

    /// `rf & 1` → creation, `rf & 2` → update.
    fn phase_of(&self, expr: &Expression<'_>) -> Option<Phase> {
        let Expression::BinaryExpression(bin) = expr.without_parentheses() else {
            return None;
        };
        if bin.operator != BinaryOperator::BitwiseAnd {
            return None;
        }
        let (flag, mask) = match (&bin.left, &bin.right) {
            (Expression::Identifier(id), mask) | (mask, Expression::Identifier(id)) => (id, mask),
            _ => return None,
        };
        let symbol = self.semantic.scoping().get_reference(flag.reference_id()).symbol_id();
        if symbol.is_none() || symbol != self.flags {
            return None;
        }
        match int_literal(mask)? {
            1 => Some(Phase::Create),
            2 => Some(Phase::Update),
            _ => None,
        }
    }

    fn expression(&mut self, expr: &'a Expression<'a>, phase: Option<Phase>) {
        match expr.without_parentheses() {
            Expression::SequenceExpression(seq) => {
                for expr in &seq.expressions {
                    self.expression(expr, phase);
                }
            }
            Expression::LogicalExpression(logical) if logical.operator == LogicalOperator::And => {
                if let Some(inner) = self.phase_of(&logical.left) {
                    self.expression(&logical.right, Some(inner));
                }
            }
            Expression::CallExpression(call) => {
                if let Some(phase) = phase {
                    self.call_chain(call, phase);
                }
            }
            _ => {}
        }
    }

    /// Records `call` (after any inner chained calls) and returns its callee:
    /// in `f(a)(b)` the outer call reuses the innermost callee.
    fn call_chain(&mut self, call: &'a CallExpression<'a>, phase: Phase) -> Option<usize> {
        let callee_index = match call.callee.without_parentheses() {
            Expression::CallExpression(inner) => self.call_chain(inner, phase)?,
            Expression::Identifier(id) => {
                let symbol = self.semantic.scoping().get_reference(id.reference_id()).symbol_id();
                self.intern(match symbol {
                    Some(symbol) if !id.name.starts_with("ɵɵ") => Callee::Symbol(symbol),
                    _ => Callee::Name(id.name.to_string()),
                })
            }
            Expression::StaticMemberExpression(member) => self.intern(Callee::Name(member.property.name.to_string())),
            _ => return None,
        };
        self.push(
            Call {
                callee_index,
                args: &call.arguments,
            },
            phase,
        );
        Some(callee_index)
    }

    fn push(&mut self, call: Call<'a>, phase: Phase) {
        match phase {
            Phase::Create => self.out.create.push(call),
            Phase::Update => self.out.update.push(call),
        }
    }

    fn intern(&mut self, callee: Callee) -> usize {
        let next = self.callees.len();
        *self.callee_index.entry(callee.clone()).or_insert_with(|| {
            self.callees.push(callee);
            next
        })
    }
}

#[derive(Default)]
struct PipeSlots(FxHashSet<u32>);

impl<'a> Visit<'a> for PipeSlots {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let int = |index: usize| {
            call.arguments
                .get(index)
                .and_then(Argument::as_expression)
                .and_then(int_literal)
        };
        if let (Some(slot), Some(_)) = (int(0), int(1)) {
            self.0.insert(slot);
        }
        walk::walk_call_expression(self, call);
    }
}
