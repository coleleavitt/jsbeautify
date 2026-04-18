//! CFF (Control-Flow-Flattening) dispatcher detector.
//!
//! Scans a JavaScript AST for dispatcher functions — functions with exactly
//! 2 parameters whose body is a single `switch` statement on the first
//! parameter, with `case IDENT: { ... } break;` clauses.
//!
//! This is a read-only scan (no AST mutation). It builds a [`DispatcherMap`]
//! that maps dispatcher names to their [`DispatcherInfo`] metadata.

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{BindingPattern, Expression, Function, Program, Statement, VariableDeclarator};
use rustc_hash::FxHashMap;

fn cff_dbg_enabled() -> bool {
    std::env::var("AST_CFF_DEBUG").is_ok()
}

macro_rules! cff_dbg {
    ($($arg:tt)*) => {
        if cff_dbg_enabled() { eprintln!($($arg)*); }
    };
}

/// Maps dispatcher function name → info about its cases.
pub type DispatcherMap = FxHashMap<String, DispatcherInfo>;

/// Metadata about a single CFF dispatcher function.
#[derive(Debug)]
pub struct DispatcherInfo {
    pub name: String,
    pub alt_name: Option<String>,
    pub state_param: String,
    pub args_param: String,
    pub cases: FxHashMap<String, CaseInfo>,
    pub has_default: bool,
}

/// Metadata about a single case clause inside a dispatcher.
#[derive(Debug)]
pub struct CaseInfo {
    pub state_name: String,
    pub body_statement_count: usize,
}

/// Read-only scanner that detects CFF dispatcher functions.
#[derive(Debug, Default)]
pub struct DispatcherDetector {
    dispatchers: DispatcherMap,
}

impl DispatcherDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan the program and return all detected dispatchers.
    #[must_use]
    pub fn detect(mut self, program: &Program<'_>) -> DispatcherMap {
        cff_dbg!(
            "[DBG/disp] detect: starting walk of {} top-level statements",
            program.body.len()
        );
        walk_stmts(&program.body, &mut self.dispatchers);
        cff_dbg!(
            "[DBG/disp] detect: finished, found {} dispatchers",
            self.dispatchers.len()
        );
        self.dispatchers
    }
}

fn walk_stmts<'a>(stmts: &OxcVec<'a, Statement<'a>>, map: &mut DispatcherMap) {
    for stmt in stmts {
        walk_stmt(stmt, map);
    }
}

fn walk_stmt<'a>(stmt: &Statement<'a>, map: &mut DispatcherMap) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            let name = func.id.as_ref().map_or("<anon>", |id| id.name.as_str());
            cff_dbg!(
                "[DBG/disp] walk_stmt: FunctionDeclaration name={} params={}",
                name,
                func.params.items.len()
            );
            if let Some(info) = try_extract_dispatcher(func, None) {
                eprintln!(
                    "[DISPATCHER] found {}({}, {}) with {} cases",
                    info.name,
                    info.state_param,
                    info.args_param,
                    info.cases.len()
                );
                map.insert(info.name.clone(), info);
            }
            if let Some(body) = &func.body {
                cff_dbg!(
                    "[DBG/disp] walk_stmt: recursing into FunctionDeclaration {} body ({} stmts)",
                    name,
                    body.statements.len()
                );
                walk_stmts(&body.statements, map);
            }
        }
        Statement::VariableDeclaration(decl) => {
            cff_dbg!(
                "[DBG/disp] walk_stmt: VariableDeclaration n={}",
                decl.declarations.len()
            );
            for d in &decl.declarations {
                check_var_declarator(d, map);
            }
        }
        Statement::ExpressionStatement(es) => {
            walk_expr(&es.expression, map);
        }
        Statement::BlockStatement(b) => {
            cff_dbg!("[DBG/disp] walk_stmt: BlockStatement n={}", b.body.len());
            walk_stmts(&b.body, map);
        }
        Statement::IfStatement(ifs) => {
            cff_dbg!("[DBG/disp] walk_stmt: IfStatement");
            walk_stmt(&ifs.consequent, map);
            if let Some(alt) = &ifs.alternate {
                walk_stmt(alt, map);
            }
        }
        Statement::TryStatement(t) => {
            cff_dbg!("[DBG/disp] walk_stmt: TryStatement");
            walk_stmts(&t.block.body, map);
            if let Some(h) = &t.handler {
                walk_stmts(&h.body.body, map);
            }
            if let Some(f) = &t.finalizer {
                walk_stmts(&f.body, map);
            }
        }
        Statement::ForStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForStatement");
            walk_stmt(&f.body, map);
        }
        Statement::WhileStatement(w) => {
            cff_dbg!("[DBG/disp] walk_stmt: WhileStatement");
            walk_stmt(&w.body, map);
        }
        Statement::DoWhileStatement(d) => {
            cff_dbg!("[DBG/disp] walk_stmt: DoWhileStatement, recursing into body");
            walk_stmt(&d.body, map);
        }
        Statement::SwitchStatement(s) => {
            cff_dbg!("[DBG/disp] walk_stmt: SwitchStatement with {} cases", s.cases.len());
            for case in &s.cases {
                walk_stmts(&case.consequent, map);
            }
        }
        Statement::ForInStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForInStatement");
            walk_stmt(&f.body, map);
        }
        Statement::ForOfStatement(f) => {
            cff_dbg!("[DBG/disp] walk_stmt: ForOfStatement");
            walk_stmt(&f.body, map);
        }
        Statement::LabeledStatement(l) => {
            cff_dbg!("[DBG/disp] walk_stmt: LabeledStatement label={}", l.label.name);
            walk_stmt(&l.body, map);
        }
        _ => {
            cff_dbg!(
                "[DBG/disp] walk_stmt: unhandled variant {:?}",
                std::mem::discriminant(stmt)
            );
        }
    }
}

/// Walk into expressions to find function expressions whose bodies may contain dispatchers.
fn walk_expr<'a>(expr: &Expression<'a>, map: &mut DispatcherMap) {
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
        Expression::ParenthesizedExpression(p) => {
            walk_expr(&p.expression, map);
        }
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
        Expression::UnaryExpression(u) => {
            walk_expr(&u.argument, map);
        }
        Expression::BinaryExpression(b) => {
            walk_expr(&b.left, map);
            walk_expr(&b.right, map);
        }
        Expression::StaticMemberExpression(s) => {
            walk_expr(&s.object, map);
        }
        Expression::ComputedMemberExpression(c) => {
            walk_expr(&c.object, map);
            walk_expr(&c.expression, map);
        }
        Expression::TaggedTemplateExpression(t) => {
            walk_expr(&t.tag, map);
        }
        _ => {}
    }
}

fn check_var_declarator(declarator: &VariableDeclarator<'_>, map: &mut DispatcherMap) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        cff_dbg!("[DBG/disp] check_var_declarator: id is not BindingIdentifier, skipping");
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        cff_dbg!(
            "[DBG/disp] check_var_declarator: {} init is not FunctionExpression",
            var_id.name
        );
        return;
    };
    let var_name = var_id.name.as_str().to_string();
    cff_dbg!(
        "[DBG/disp] check_var_declarator: {} = function {}(...) with {} params",
        var_name,
        func.id.as_ref().map_or("<anon>", |id| id.name.as_str()),
        func.params.items.len()
    );
    if let Some(info) = try_extract_dispatcher(func, Some(var_name.clone())) {
        eprintln!(
            "[DISPATCHER] found {}({}, {}) with {} cases (alt_name={:?})",
            info.name,
            info.state_param,
            info.args_param,
            info.cases.len(),
            info.alt_name
        );
        if let Some(alt) = &info.alt_name {
            map.insert(
                alt.clone(),
                DispatcherInfo {
                    name: alt.clone(),
                    alt_name: Some(info.name.clone()),
                    state_param: info.state_param.clone(),
                    args_param: info.args_param.clone(),
                    cases: FxHashMap::default(),
                    has_default: info.has_default,
                },
            );
        }
        map.insert(info.name.clone(), info);
    }
    // Always recurse into the function body to find nested dispatchers
    if let Some(body) = &func.body {
        cff_dbg!(
            "[DBG/disp] check_var_declarator: recursing into {} body ({} stmts)",
            var_id.name,
            body.statements.len()
        );
        walk_stmts(&body.statements, map);
    }
}

fn try_extract_dispatcher(func: &Function<'_>, var_name: Option<String>) -> Option<DispatcherInfo> {
    let fn_label = var_name
        .as_deref()
        .or_else(|| func.id.as_ref().map(|id| id.name.as_str()))
        .unwrap_or("<anon>");

    if func.r#async || func.generator {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: async={} generator={}",
            fn_label,
            func.r#async,
            func.generator
        );
        return None;
    }
    if func.params.items.len() != 2 {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params.len()={} (need 2)",
            fn_label,
            func.params.items.len()
        );
        return None;
    }

    let BindingPattern::BindingIdentifier(p1) = &func.params.items[0].pattern else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params[0] not BindingIdentifier",
            fn_label
        );
        return None;
    };
    let BindingPattern::BindingIdentifier(p2) = &func.params.items[1].pattern else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: params[1] not BindingIdentifier",
            fn_label
        );
        return None;
    };

    let body = func.body.as_ref()?;

    // Allow var decls / expr stmts before the switch (e.g., `var xF = rQ; switch(...)`)
    let switch = body.statements.iter().find_map(|s| {
        if let Statement::SwitchStatement(sw) = s {
            Some(sw)
        } else {
            None
        }
    });
    let Some(switch) = switch else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: no SwitchStatement in {} body stmts",
            fn_label,
            body.statements.len()
        );
        return None;
    };
    // Reject bodies with control flow beyond var/expr/switch (prevents false positives)
    let non_switch_non_var = body
        .statements
        .iter()
        .filter(|s| {
            !matches!(
                s,
                Statement::SwitchStatement(_) | Statement::VariableDeclaration(_) | Statement::ExpressionStatement(_)
            )
        })
        .count();
    if non_switch_non_var > 0 {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: {} non-switch/non-var/non-expr stmts in body",
            fn_label,
            non_switch_non_var
        );
        return None;
    }

    let Expression::Identifier(disc) = &switch.discriminant else {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: discriminant is not Identifier",
            fn_label
        );
        return None;
    };
    if disc.name.as_str() != p1.name.as_str() {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: discriminant '{}' != p1 '{}'",
            fn_label,
            disc.name,
            p1.name
        );
        return None;
    }

    let mut cases = FxHashMap::default();
    let mut has_default = false;

    for case in &switch.cases {
        match &case.test {
            Some(Expression::Identifier(id)) => {
                let label = id.name.as_str().to_string();
                let body_count = case
                    .consequent
                    .iter()
                    .filter(|s| !matches!(s, Statement::BreakStatement(_)))
                    .count();
                cases.insert(
                    label.clone(),
                    CaseInfo {
                        state_name: label,
                        body_statement_count: body_count,
                    },
                );
            }
            None => {
                has_default = true;
            }
            _ => {
                cff_dbg!(
                    "[DBG/disp] try_extract {}: case test is not Identifier (skipping case)",
                    fn_label
                );
            }
        }
    }

    if cases.is_empty() {
        cff_dbg!(
            "[DBG/disp] try_extract {}: rejected: no valid cases found (switch has {} cases)",
            fn_label,
            switch.cases.len()
        );
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

    cff_dbg!("[DBG/disp] try_extract {}: SUCCESS with {} cases", name, cases.len());

    Some(DispatcherInfo {
        name,
        alt_name,
        state_param: p1.name.as_str().to_string(),
        args_param: p2.name.as_str().to_string(),
        cases,
        has_default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn detects_simple_dispatcher() {
        let code = r#"
            function F(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                    case Y: { bar(); } break;
                }
            }
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DispatcherDetector::new();
        let map = detector.detect(&ret.program);

        assert_eq!(map.len(), 1);
        let info = map.get("F").expect("should detect F");
        assert_eq!(info.state_param, "s");
        assert_eq!(info.args_param, "a");
        assert_eq!(info.cases.len(), 2);
        assert!(info.cases.contains_key("X"));
        assert!(info.cases.contains_key("Y"));
        assert!(!info.has_default);
        assert!(info.alt_name.is_none());
    }

    #[test]
    fn detects_var_assigned_dispatcher() {
        let code = r#"
            var F = function G(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                }
            };
        "#;
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DispatcherDetector::new();
        let map = detector.detect(&ret.program);

        let info = map.get("F").expect("should detect F");
        assert_eq!(info.name, "F");
        assert_eq!(info.alt_name.as_deref(), Some("G"));
        assert_eq!(info.cases.len(), 1);
        assert!(info.cases.contains_key("X"));
    }

    #[test]
    fn ignores_non_dispatcher() {
        let code = "function f(x) { return x + 1; }";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let detector = DispatcherDetector::new();
        let map = detector.detect(&ret.program);

        assert!(map.is_empty());
    }
}
