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
        walk_stmts(&program.body, &mut self.dispatchers);
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
                walk_stmts(&body.statements, map);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                check_var_declarator(d, map);
            }
        }
        Statement::ExpressionStatement(es) => {
            if let Expression::CallExpression(call) = &es.expression {
                let callee = match &call.callee {
                    Expression::ParenthesizedExpression(p) => &p.expression,
                    other => other,
                };
                if let Expression::FunctionExpression(func) = callee {
                    if let Some(body) = &func.body {
                        walk_stmts(&body.statements, map);
                    }
                }
            }
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
        _ => {}
    }
}

fn check_var_declarator(declarator: &VariableDeclarator<'_>, map: &mut DispatcherMap) {
    let BindingPattern::BindingIdentifier(var_id) = &declarator.id else {
        return;
    };
    let Some(Expression::FunctionExpression(func)) = &declarator.init else {
        return;
    };
    let var_name = var_id.name.as_str().to_string();
    if let Some(info) = try_extract_dispatcher(func, Some(var_name)) {
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
}

fn try_extract_dispatcher(func: &Function<'_>, var_name: Option<String>) -> Option<DispatcherInfo> {
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
    if body.statements.len() != 1 {
        return None;
    }

    let Statement::SwitchStatement(switch) = &body.statements[0] else {
        return None;
    };

    let Expression::Identifier(disc) = &switch.discriminant else {
        return None;
    };
    if disc.name.as_str() != p1.name.as_str() {
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
            _ => {}
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
