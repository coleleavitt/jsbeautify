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
    VariableDeclarator,
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

#[derive(Debug, PartialEq, Eq)]
pub enum StateTransition {
    Sequential(String),
    Conditional(String, String),
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
}
