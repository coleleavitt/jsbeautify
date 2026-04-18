//! Do-while-switch dispatcher detector.
//!
//! Scans a JavaScript AST for dispatcher functions of the form:
//! `function F(STATE, ARGS) { do { switch(STATE) { ... } } while (STATE != SENTINEL); }`
//!
//! This is a read-only scan (no AST mutation). It builds a map of dispatcher
//! names to their [`DoWhileDispatcherInfo`] metadata.

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BinaryOperator, BindingPattern, Expression, Function, Program, Statement,
    UnaryOperator, VariableDeclarator,
};
use rustc_hash::FxHashMap;

fn dwsd_dbg_enabled() -> bool {
    std::env::var("AST_DWSD_DEBUG").is_ok()
}

macro_rules! dwsd_dbg {
    ($($arg:tt)*) => {
        if dwsd_dbg_enabled() { eprintln!($($arg)*); }
    };
}

pub type DoWhileDispatcherMap = FxHashMap<String, DoWhileDispatcherInfo>;

#[derive(Debug)]
pub struct DoWhileDispatcherInfo {
    pub name: String,
    pub alt_name: Option<String>,
    pub state_param: String,
    pub args_param: String,
    pub exit_sentinel: String,
    pub cases: Vec<DoWhileCaseInfo>,
    pub has_default: bool,
}

#[derive(Debug)]
pub struct DoWhileCaseInfo {
    pub label: String,
    pub body_statement_count: usize,
    pub transition: StateTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    AddAssign,
    SubAssign,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StateTransition {
    Sequential(String),
    Conditional(String, String),
    Compound { op: CompoundOp, rhs_name: String },
    Return,
    LoopExit,
    Unknown,
}

#[derive(Debug, Default)]
pub struct DoWhileSwitchDetector {
    dispatchers: DoWhileDispatcherMap,
}

impl DoWhileSwitchDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn detect(mut self, program: &Program<'_>) -> DoWhileDispatcherMap {
        dwsd_dbg!(
            "[DBG/dwsd] detect: starting walk of {} top-level statements",
            program.body.len()
        );
        walk_stmts(&program.body, &mut self.dispatchers);
        dwsd_dbg!(
            "[DBG/dwsd] detect: finished, found {} dispatchers",
            self.dispatchers.len()
        );
        self.dispatchers
    }
}

fn walk_stmts<'a>(stmts: &OxcVec<'a, Statement<'a>>, map: &mut DoWhileDispatcherMap) {
    for stmt in stmts {
        walk_stmt(stmt, map);
    }
}

fn walk_stmt<'a>(stmt: &Statement<'a>, map: &mut DoWhileDispatcherMap) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            if let Some(info) = try_extract_dowhile_dispatcher(func, None) {
                insert_dispatcher(map, info);
            }
            if let Some(body) = &func.body {
                walk_stmts(&body.statements, map);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                check_var_declarator(d, map);
            }
        }
        Statement::ExpressionStatement(es) => {
            walk_expr(&es.expression, map);
        }
        Statement::BlockStatement(b) => walk_stmts(&b.body, map),
        Statement::IfStatement(ifs) => {
            walk_stmt(&ifs.consequent, map);
            if let Some(alt) = &ifs.alternate {
                walk_stmt(alt, map);
            }
        }
        Statement::TryStatement(t) => {
            walk_stmts(&t.block.body, map);
            if let Some(h) = &t.handler {
                walk_stmts(&h.body.body, map);
            }
            if let Some(f) = &t.finalizer {
                walk_stmts(&f.body, map);
            }
        }
        Statement::ForStatement(f) => walk_stmt(&f.body, map),
        Statement::WhileStatement(w) => walk_stmt(&w.body, map),
        Statement::DoWhileStatement(d) => walk_stmt(&d.body, map),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                walk_stmts(&case.consequent, map);
            }
        }
        Statement::ForInStatement(f) => walk_stmt(&f.body, map),
        Statement::ForOfStatement(f) => walk_stmt(&f.body, map),
        Statement::LabeledStatement(l) => walk_stmt(&l.body, map),
        _ => {}
    }
}

fn walk_expr<'a>(expr: &Expression<'a>, map: &mut DoWhileDispatcherMap) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                walk_stmts(&body.statements, map);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            walk_stmts(&func.body.statements, map);
        }
        Expression::AssignmentExpression(assign) => {
            walk_expr(&assign.right, map);
        }
        Expression::CallExpression(call) => {
            walk_expr(&call.callee, map);
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::NewExpression(ne) => {
            walk_expr(&ne.callee, map);
            for arg in &ne.arguments {
                if let Some(e) = arg.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                walk_expr(e, map);
            }
        }
        Expression::ParenthesizedExpression(p) => walk_expr(&p.expression, map),
        Expression::ConditionalExpression(c) => {
            walk_expr(&c.consequent, map);
            walk_expr(&c.alternate, map);
        }
        Expression::LogicalExpression(l) => {
            walk_expr(&l.left, map);
            walk_expr(&l.right, map);
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    walk_expr(&p.value, map);
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    walk_expr(e, map);
                }
            }
        }
        Expression::UnaryExpression(u) => walk_expr(&u.argument, map),
        Expression::BinaryExpression(b) => {
            walk_expr(&b.left, map);
            walk_expr(&b.right, map);
        }
        Expression::StaticMemberExpression(s) => walk_expr(&s.object, map),
        Expression::ComputedMemberExpression(c) => {
            walk_expr(&c.object, map);
            walk_expr(&c.expression, map);
        }
        Expression::TaggedTemplateExpression(t) => walk_expr(&t.tag, map),
        _ => {}
    }
}

fn check_var_declarator(declarator: &VariableDeclarator<'_>, map: &mut DoWhileDispatcherMap) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        if let Some(init) = &declarator.init {
            walk_expr(init, map);
        }
        return;
    };
    let var_name = var_id.name.as_str().to_string();
    if let Some(info) = try_extract_dowhile_dispatcher(func, Some(var_name.clone())) {
        insert_dispatcher(map, info);
    }
    if let Some(body) = &func.body {
        walk_stmts(&body.statements, map);
    }
}

fn insert_dispatcher(map: &mut DoWhileDispatcherMap, info: DoWhileDispatcherInfo) {
    if let Some(alt) = &info.alt_name {
        map.insert(
            alt.clone(),
            DoWhileDispatcherInfo {
                name: alt.clone(),
                alt_name: Some(info.name.clone()),
                state_param: info.state_param.clone(),
                args_param: info.args_param.clone(),
                exit_sentinel: info.exit_sentinel.clone(),
                cases: Vec::new(),
                has_default: info.has_default,
            },
        );
    }
    map.insert(info.name.clone(), info);
}

fn try_extract_dowhile_dispatcher(func: &Function<'_>, var_name: Option<String>) -> Option<DoWhileDispatcherInfo> {
    let fn_label = var_name
        .as_deref()
        .or_else(|| func.id.as_ref().map(|id| id.name.as_str()))
        .unwrap_or("<anon>");

    if func.r#async || func.generator {
        return None;
    }
    if func.params.items.len() != 2 {
        return None;
    }

    let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
        return None;
    };
    let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
        return None;
    };

    let body = func.body.as_ref()?;
    let state_param = p1.name.as_str();

    let dowhile = body.statements.iter().find_map(|s| {
        if let Statement::DoWhileStatement(dw) = s {
            Some(dw)
        } else {
            None
        }
    })?;

    let Expression::BinaryExpression(test_bin) = &dowhile.test else {
        dwsd_dbg!(
            "[DBG/dwsd] try_extract {}: do-while test is not BinaryExpression",
            fn_label
        );
        return None;
    };
    if !matches!(
        test_bin.operator,
        BinaryOperator::Inequality | BinaryOperator::StrictInequality
    ) {
        dwsd_dbg!(
            "[DBG/dwsd] try_extract {}: do-while test operator is not != or !==",
            fn_label
        );
        return None;
    }
    let Expression::Identifier(test_left) = &test_bin.left else {
        return None;
    };
    if test_left.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(test_right) = &test_bin.right else {
        return None;
    };
    let exit_sentinel = test_right.name.as_str().to_string();

    let switch = extract_switch_from_dowhile_body(&dowhile.body, state_param)?;

    let mut cases = Vec::new();
    let mut has_default = false;

    for case in &switch.cases {
        match &case.test {
            Some(Expression::Identifier(id)) => {
                let label = id.name.as_str().to_string();
                let non_break: Vec<_> = case
                    .consequent
                    .iter()
                    .filter(|s| !matches!(s, Statement::BreakStatement(_)))
                    .collect();
                let body_count = non_break.len();
                let transition = classify_transition(&non_break, state_param, &exit_sentinel);
                cases.push(DoWhileCaseInfo {
                    label,
                    body_statement_count: body_count,
                    transition,
                });
            }
            None => {
                has_default = true;
            }
            _ => {
                dwsd_dbg!("[DBG/dwsd] try_extract {}: case test is not Identifier", fn_label);
            }
        }
    }

    if cases.is_empty() {
        return None;
    }

    let (name, alt_name) = match var_name {
        Some(vn) => {
            let fn_name = func.id.as_ref().map(|id| id.name.as_str().to_string());
            (vn, fn_name)
        }
        None => {
            let fn_name = func.id.as_ref()?.name.as_str().to_string();
            (fn_name, None)
        }
    };

    dwsd_dbg!(
        "[DBG/dwsd] try_extract {}: SUCCESS with {} cases, exit_sentinel={}",
        name,
        cases.len(),
        exit_sentinel
    );

    Some(DoWhileDispatcherInfo {
        name,
        alt_name,
        state_param: state_param.to_string(),
        args_param: p2.name.as_str().to_string(),
        exit_sentinel,
        cases,
        has_default,
    })
}

fn extract_switch_from_dowhile_body<'a, 'b>(
    body: &'b Statement<'a>,
    state_param: &str,
) -> Option<&'b oxc_ast::ast::SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => {
            let Expression::Identifier(disc) = &sw.discriminant else {
                return None;
            };
            if disc.name.as_str() == state_param {
                Some(sw)
            } else {
                None
            }
        }
        Statement::BlockStatement(block) => block.body.iter().find_map(|s| {
            if let Statement::SwitchStatement(sw) = s {
                let Expression::Identifier(disc) = &sw.discriminant else {
                    return None;
                };
                if disc.name.as_str() == state_param {
                    Some(sw.as_ref())
                } else {
                    None
                }
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn classify_transition(stmts: &[&Statement<'_>], state_param: &str, exit_sentinel: &str) -> StateTransition {
    if stmts.is_empty() {
        return StateTransition::Unknown;
    }

    let last = stmts[stmts.len() - 1];
    if has_return(last) {
        return StateTransition::Return;
    }

    if let Some(transition) = check_if_conditional(last, state_param) {
        return transition;
    }

    for stmt in stmts.iter().rev() {
        if let Some(next_state) = extract_state_assignment(stmt, state_param) {
            if next_state == exit_sentinel {
                return StateTransition::LoopExit;
            }
            return StateTransition::Sequential(next_state);
        }
        if let Some(compound) = extract_compound_assignment(stmt, state_param) {
            return compound;
        }
    }

    StateTransition::Unknown
}

fn has_return(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(block) => block.body.iter().any(has_return),
        _ => false,
    }
}

fn extract_state_assignment<'a>(stmt: &Statement<'a>, state_param: &str) -> Option<String> {
    let Statement::ExpressionStatement(es) = stmt else {
        if let Statement::BlockStatement(block) = stmt {
            for s in block.body.iter().rev() {
                if let Some(name) = extract_state_assignment(s, state_param) {
                    return Some(name);
                }
            }
        }
        return None;
    };
    let Expression::AssignmentExpression(assign) = &es.expression else {
        return None;
    };
    if assign.operator != AssignmentOperator::Assign {
        return None;
    }
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    if target.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(rhs) = &assign.right else {
        return None;
    };
    Some(rhs.name.as_str().to_string())
}

fn extract_compound_assignment(stmt: &Statement<'_>, state_param: &str) -> Option<StateTransition> {
    let Statement::ExpressionStatement(es) = stmt else {
        if let Statement::BlockStatement(block) = stmt {
            for s in block.body.iter().rev() {
                if let Some(t) = extract_compound_assignment(s, state_param) {
                    return Some(t);
                }
            }
        }
        return None;
    };
    let Expression::AssignmentExpression(assign) = &es.expression else {
        return None;
    };
    let op = match assign.operator {
        AssignmentOperator::Addition => CompoundOp::AddAssign,
        AssignmentOperator::Subtraction => CompoundOp::SubAssign,
        _ => return None,
    };
    let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
        return None;
    };
    if target.name.as_str() != state_param {
        return None;
    }
    let Expression::Identifier(rhs) = &assign.right else {
        return None;
    };
    Some(StateTransition::Compound {
        op,
        rhs_name: rhs.name.as_str().to_string(),
    })
}

fn check_if_conditional(stmt: &Statement<'_>, state_param: &str) -> Option<StateTransition> {
    let stmt = if let Statement::BlockStatement(block) = stmt {
        block.body.last()?
    } else {
        stmt
    };

    let Statement::IfStatement(ifs) = stmt else {
        return None;
    };

    let consequent_state = extract_state_from_branch(&ifs.consequent, state_param);
    let alternate_state = ifs
        .alternate
        .as_ref()
        .and_then(|alt| extract_state_from_branch(alt, state_param));

    match (consequent_state, alternate_state) {
        (Some(a), Some(b)) => Some(StateTransition::Conditional(a, b)),
        _ => None,
    }
}

fn extract_state_from_branch(stmt: &Statement<'_>, state_param: &str) -> Option<String> {
    match stmt {
        Statement::ExpressionStatement(_) => extract_state_assignment(stmt, state_param),
        Statement::BlockStatement(block) => {
            for s in block.body.iter().rev() {
                if let Some(name) = extract_state_assignment(s, state_param) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn try_eval_expr(expr: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) => {
            let v = n.value;
            if v.fract() != 0.0 || v > i64::MAX as f64 || v < i64::MIN as f64 {
                return None;
            }
            Some(v as i64)
        }
        Expression::Identifier(id) => known.get(id.name.as_str()).copied(),
        Expression::BinaryExpression(bin) => {
            if bin.operator == BinaryOperator::Addition {
                if let Some(concat) = try_eval_array_concat(&bin.left, &bin.right, known) {
                    return Some(concat);
                }
            }
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
        Expression::ArrayExpression(arr) => {
            if arr.elements.is_empty() {
                return Some(0);
            }
            if arr.elements.len() == 1 {
                if let Some(e) = arr.elements[0].as_expression() {
                    return try_eval_expr(e, known);
                }
            }
            None
        }
        _ => None,
    }
}

/// Handles JS `[N] + [M]` → string concat → numeric: `"NM"` parsed as i64.
fn try_eval_array_concat(left: &Expression<'_>, right: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<i64> {
    let l_str = try_eval_array_as_string(left, known)?;
    let r_str = try_eval_array_as_string(right, known)?;
    let concat = format!("{l_str}{r_str}");
    concat.parse::<i64>().ok()
}

fn try_eval_array_as_string(expr: &Expression<'_>, known: &FxHashMap<String, i64>) -> Option<String> {
    let Expression::ArrayExpression(arr) = expr else {
        return None;
    };
    if arr.elements.is_empty() {
        return Some(String::new());
    }
    if arr.elements.len() == 1 {
        if let Some(e) = arr.elements[0].as_expression() {
            let v = try_eval_expr(e, known)?;
            return Some(v.to_string());
        }
    }
    None
}

enum DeferredConst {
    Lit(i64),
    Ident(String),
    Add(Box<DeferredConst>, Box<DeferredConst>),
    Sub(Box<DeferredConst>, Box<DeferredConst>),
    Mul(Box<DeferredConst>, Box<DeferredConst>),
    Array(Option<Box<DeferredConst>>),
    ArrayConcat(Box<DeferredConst>, Box<DeferredConst>),
}

impl DeferredConst {
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
                if bin.operator == BinaryOperator::Addition {
                    if let (Some(l_arr), Some(r_arr)) =
                        (Self::from_array_ast(&bin.left), Self::from_array_ast(&bin.right))
                    {
                        return Some(Self::ArrayConcat(Box::new(l_arr), Box::new(r_arr)));
                    }
                }
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
            Expression::ArrayExpression(arr) => {
                if arr.elements.is_empty() {
                    Some(Self::Array(None))
                } else if arr.elements.len() == 1 {
                    if let Some(e) = arr.elements[0].as_expression() {
                        Some(Self::Array(Some(Box::new(Self::from_ast(e)?))))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn from_array_ast(expr: &Expression<'_>) -> Option<Self> {
        let Expression::ArrayExpression(arr) = expr else {
            return None;
        };
        if arr.elements.is_empty() {
            Some(Self::Array(None))
        } else if arr.elements.len() == 1 {
            if let Some(e) = arr.elements[0].as_expression() {
                Some(Self::Array(Some(Box::new(Self::from_ast(e)?))))
            } else {
                None
            }
        } else {
            None
        }
    }

    fn eval(&self, known: &FxHashMap<String, i64>) -> Option<i64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Ident(name) => known.get(name.as_str()).copied(),
            Self::Add(l, r) => l.eval(known)?.checked_add(r.eval(known)?),
            Self::Sub(l, r) => l.eval(known)?.checked_sub(r.eval(known)?),
            Self::Mul(l, r) => l.eval(known)?.checked_mul(r.eval(known)?),
            Self::Array(None) => Some(0),
            Self::Array(Some(inner)) => inner.eval(known),
            Self::ArrayConcat(l, r) => {
                let l_str = l.eval_as_string(known)?;
                let r_str = r.eval_as_string(known)?;
                format!("{l_str}{r_str}").parse::<i64>().ok()
            }
        }
    }

    fn eval_as_string(&self, known: &FxHashMap<String, i64>) -> Option<String> {
        match self {
            Self::Array(None) => Some(String::new()),
            Self::Array(Some(inner)) => inner.eval(known).map(|v| v.to_string()),
            _ => None,
        }
    }
}

/// Collect numeric constants from `var X = EXPR;` and `X = EXPR;` assignments.
pub fn collect_constants(program: &Program<'_>) -> FxHashMap<String, i64> {
    let mut known: FxHashMap<String, i64> = FxHashMap::default();
    let mut deferred: Vec<(String, DeferredConst)> = Vec::new();

    collect_constants_from_stmts(&program.body, &mut known, &mut deferred);

    for _ in 0..10 {
        let prev = known.len();
        deferred.retain(|(name, expr)| {
            if let Some(v) = expr.eval(&known) {
                known.insert(name.clone(), v);
                return false;
            }
            true
        });
        if known.len() == prev {
            break;
        }
    }

    known
}

fn collect_constants_from_stmts<'a>(
    stmts: &OxcVec<'a, Statement<'a>>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    for stmt in stmts {
        collect_constants_from_stmt(stmt, known, deferred);
    }
}

fn collect_constants_from_stmt<'a>(
    stmt: &Statement<'a>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    if let Some(init) = &d.init {
                        collect_constants_from_expr(init, known, deferred);
                    }
                    continue;
                };
                let Some(init) = &d.init else { continue };
                let name = id.name.as_str().to_string();
                if let Some(v) = try_eval_expr(init, known) {
                    known.insert(name, v);
                } else if let Some(dc) = DeferredConst::from_ast(init) {
                    deferred.push((name, dc));
                } else {
                    collect_constants_from_expr(init, known, deferred);
                }
            }
        }
        Statement::ExpressionStatement(es) => {
            if let Expression::AssignmentExpression(assign) = &es.expression {
                if let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left {
                    let name = target.name.as_str().to_string();
                    if assign.operator == AssignmentOperator::Assign {
                        if let Some(v) = try_eval_expr(&assign.right, known) {
                            known.insert(name, v);
                        } else if let Some(dc) = DeferredConst::from_ast(&assign.right) {
                            deferred.push((name, dc));
                        } else {
                            collect_constants_from_expr(&assign.right, known, deferred);
                        }
                    }
                } else {
                    collect_constants_from_expr(&es.expression, known, deferred);
                }
            } else {
                collect_constants_from_expr(&es.expression, known, deferred);
            }
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(body) = &func.body {
                collect_constants_from_stmts(&body.statements, known, deferred);
            }
        }
        Statement::BlockStatement(b) => collect_constants_from_stmts(&b.body, known, deferred),
        Statement::IfStatement(ifs) => {
            collect_constants_from_stmt(&ifs.consequent, known, deferred);
            if let Some(alt) = &ifs.alternate {
                collect_constants_from_stmt(alt, known, deferred);
            }
        }
        Statement::TryStatement(t) => {
            collect_constants_from_stmts(&t.block.body, known, deferred);
            if let Some(h) = &t.handler {
                collect_constants_from_stmts(&h.body.body, known, deferred);
            }
            if let Some(f) = &t.finalizer {
                collect_constants_from_stmts(&f.body, known, deferred);
            }
        }
        Statement::ForStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::WhileStatement(w) => collect_constants_from_stmt(&w.body, known, deferred),
        Statement::DoWhileStatement(d) => collect_constants_from_stmt(&d.body, known, deferred),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                collect_constants_from_stmts(&case.consequent, known, deferred);
            }
        }
        Statement::ForInStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::ForOfStatement(f) => collect_constants_from_stmt(&f.body, known, deferred),
        Statement::LabeledStatement(l) => collect_constants_from_stmt(&l.body, known, deferred),
        _ => {}
    }
}

fn collect_constants_from_expr<'a>(
    expr: &Expression<'a>,
    known: &mut FxHashMap<String, i64>,
    deferred: &mut Vec<(String, DeferredConst)>,
) {
    match expr {
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                collect_constants_from_stmts(&body.statements, known, deferred);
            }
        }
        Expression::ArrowFunctionExpression(func) => {
            collect_constants_from_stmts(&func.body.statements, known, deferred);
        }
        Expression::AssignmentExpression(assign) => {
            collect_constants_from_expr(&assign.right, known, deferred);
        }
        Expression::CallExpression(call) => {
            collect_constants_from_expr(&call.callee, known, deferred);
        }
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_constants_from_expr(e, known, deferred);
            }
        }
        Expression::ParenthesizedExpression(p) => {
            collect_constants_from_expr(&p.expression, known, deferred);
        }
        _ => {}
    }
}

/// Resolve `Compound` transitions to `Sequential` using a constant dictionary.
pub fn resolve_compound_transitions(info: &mut DoWhileDispatcherInfo, constants: &FxHashMap<String, i64>) -> usize {
    let mut label_values: FxHashMap<String, i64> = FxHashMap::default();
    for case in &info.cases {
        if let Some(&val) = constants.get(&case.label) {
            label_values.insert(case.label.clone(), val);
        }
    }

    let mut value_to_label: FxHashMap<i64, String> = FxHashMap::default();
    for (label, &val) in &label_values {
        value_to_label.insert(val, label.clone());
    }

    let mut resolved = 0;
    for case in &mut info.cases {
        if let StateTransition::Compound { op, ref rhs_name } = case.transition {
            let Some(&src_val) = label_values.get(&case.label) else {
                continue;
            };
            let Some(&rhs_val) = constants.get(rhs_name.as_str()) else {
                continue;
            };
            let target_val = match op {
                CompoundOp::AddAssign => src_val.wrapping_add(rhs_val),
                CompoundOp::SubAssign => src_val.wrapping_sub(rhs_val),
            };
            if let Some(target_label) = value_to_label.get(&target_val) {
                case.transition = StateTransition::Sequential(target_label.clone());
                resolved += 1;
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn detects_simple_dowhile_switch() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { s = Y; } break;
                        case Y: { return 1; } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        assert_eq!(map.len(), 1);
        let info = map.get("F").expect("should detect F");
        assert_eq!(info.state_param, "s");
        assert_eq!(info.args_param, "a");
        assert_eq!(info.exit_sentinel, "Z");
        assert_eq!(info.cases.len(), 2);
        assert!(!info.has_default);
    }

    #[test]
    fn detects_var_assigned_dowhile() {
        let code = r#"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case X: { s = Y; } break;
                    }
                } while (s != Z);
            };
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.name, "F");
        assert_eq!(info.alt_name.as_deref(), Some("G"));
        assert!(map.contains_key("G"));
    }

    #[test]
    fn classifies_sequential_transition() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { foo(); s = NEXT; } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Sequential("NEXT".to_string())
        );
    }

    #[test]
    fn classifies_return_transition() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { return 42; } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(info.cases[0].transition, StateTransition::Return);
    }

    #[test]
    fn classifies_conditional_with_both_labels() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: {
                            if (a[0]) { s = A; } else { s = B; }
                        } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.cases.len(), 1);
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Conditional("A".to_string(), "B".to_string())
        );
    }

    #[test]
    fn classifies_compound_add_assign() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { foo(); s += C; } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Compound {
                op: CompoundOp::AddAssign,
                rhs_name: "C".to_string()
            }
        );
    }

    #[test]
    fn classifies_compound_sub_assign() {
        let code = r#"
            function F(s, a) {
                do {
                    switch (s) {
                        case X: { bar(); s -= D; } break;
                    }
                } while (s != Z);
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DoWhileSwitchDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(
            info.cases[0].transition,
            StateTransition::Compound {
                op: CompoundOp::SubAssign,
                rhs_name: "D".to_string()
            }
        );
    }

    #[test]
    fn resolves_compound_to_sequential() {
        let mut info = DoWhileDispatcherInfo {
            name: "F".to_string(),
            alt_name: None,
            state_param: "s".to_string(),
            args_param: "a".to_string(),
            exit_sentinel: "Z".to_string(),
            cases: vec![
                DoWhileCaseInfo {
                    label: "X".to_string(),
                    body_statement_count: 1,
                    transition: StateTransition::Compound {
                        op: CompoundOp::AddAssign,
                        rhs_name: "C".to_string(),
                    },
                },
                DoWhileCaseInfo {
                    label: "Y".to_string(),
                    body_statement_count: 1,
                    transition: StateTransition::Return,
                },
            ],
            has_default: false,
        };
        let mut constants: FxHashMap<String, i64> = FxHashMap::default();
        constants.insert("X".to_string(), 10);
        constants.insert("C".to_string(), 3);
        constants.insert("Y".to_string(), 13);

        let resolved = resolve_compound_transitions(&mut info, &constants);
        assert_eq!(resolved, 1);
        assert_eq!(info.cases[0].transition, StateTransition::Sequential("Y".to_string()));
    }

    #[test]
    fn unresolved_compound_stays() {
        let mut info = DoWhileDispatcherInfo {
            name: "F".to_string(),
            alt_name: None,
            state_param: "s".to_string(),
            args_param: "a".to_string(),
            exit_sentinel: "Z".to_string(),
            cases: vec![DoWhileCaseInfo {
                label: "X".to_string(),
                body_statement_count: 1,
                transition: StateTransition::Compound {
                    op: CompoundOp::AddAssign,
                    rhs_name: "UNKNOWN".to_string(),
                },
            }],
            has_default: false,
        };
        let mut constants: FxHashMap<String, i64> = FxHashMap::default();
        constants.insert("X".to_string(), 10);

        let resolved = resolve_compound_transitions(&mut info, &constants);
        assert_eq!(resolved, 0);
        assert!(matches!(info.cases[0].transition, StateTransition::Compound { .. }));
    }
}
