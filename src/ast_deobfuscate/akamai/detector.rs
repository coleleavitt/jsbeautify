use oxc_ast::ast::{BindingPattern, Expression, Function, IdentifierReference, Statement, StringLiteral};
use oxc_traverse::Traverse;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;

mod walker;

pub(super) use walker::walk_detector;

/// Well-known BMP string/identifier signatures. Any match adds 1 signature point.
pub(super) const BMP_IDENT_SIGS: &[&str] = &[
    "bmak",
    "_abck",
    "bm_sz",
    "ak_bmsc",
    "bm_sv",
    "sensor_data",
    "akamai-bm-telemetry",
];

/// Minimum distinct signature hits required to classify a script as BMP.
pub(super) const DETECTION_THRESHOLD: usize = 2;

/// Lightweight signature scanner. Does NOT modify the AST.
#[derive(Debug, Default)]
pub struct AkamaiDetector {
    hits: FxHashSet<String>,
    self_init_accessors: usize,
    accessor_names: Vec<String>,
}

impl AkamaiDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if enough BMP signatures were observed to warrant running
    /// the Akamai-specific pass suite.
    #[must_use]
    pub fn is_akamai(&self) -> bool {
        self.hits.len() >= DETECTION_THRESHOLD || self.self_init_accessors >= 3
    }

    #[must_use]
    pub fn signature_count(&self) -> usize {
        self.hits.len()
    }

    #[must_use]
    pub const fn self_init_accessor_count(&self) -> usize {
        self.self_init_accessors
    }

    #[must_use]
    pub fn accessor_names(&self) -> &[String] {
        &self.accessor_names
    }

    #[must_use]
    pub const fn hits(&self) -> &FxHashSet<String> {
        &self.hits
    }

    pub(super) fn record(&mut self, sig: &str) {
        self.hits.insert(sig.to_string());
    }

    pub(super) fn record_accessor(&mut self, name: &str) {
        self.self_init_accessors += 1;
        if self.accessor_names.len() < 16 {
            self.accessor_names.push(name.to_string());
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for AkamaiDetector {
    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, _ctx: &mut Ctx<'a>) {
        let name = ident.name.as_str();
        if BMP_IDENT_SIGS.contains(&name) {
            self.record(name);
        }
    }

    fn enter_string_literal(&mut self, lit: &mut StringLiteral<'a>, _ctx: &mut Ctx<'a>) {
        let value = lit.value.as_str();
        if BMP_IDENT_SIGS.contains(&value) {
            self.record(value);
        }
    }

    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if is_self_init_accessor(func)
            && let Some(name) = func.id.as_ref().map(|id| id.name.as_str())
        {
            self.record_accessor(name);
        }
    }
}

/// Detects the pattern:
/// ```ignore
/// function F() {
///     var x = <init>;        // [] | {} | Object.create(...) | new Object() | [].entries()
///     F = function () { return x; };
///     return x;
/// }
/// ```
pub(super) fn is_self_init_accessor(func: &Function<'_>) -> bool {
    let Some(name) = func.id.as_ref().map(|id| id.name.as_str()) else {
        return false;
    };
    if !func.params.items.is_empty() {
        return false;
    }
    let Some(body) = func.body.as_ref() else {
        return false;
    };
    if body.statements.len() != 3 {
        return false;
    }

    // Statement 1: var <local> = <init>;
    let Statement::VariableDeclaration(decl) = &body.statements[0] else {
        return false;
    };
    if decl.declarations.len() != 1 {
        return false;
    }
    let local_name = match &decl.declarations[0].id {
        BindingPattern::BindingIdentifier(id) => id.name.as_str(),
        _ => return false,
    };

    // Statement 2: F = function () { return <local>; };
    let Statement::ExpressionStatement(expr_stmt) = &body.statements[1] else {
        return false;
    };
    let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
        return false;
    };
    let Some(target_name) = assignment_target_name(&assign.left) else {
        return false;
    };
    if target_name != name {
        return false;
    }
    let Expression::FunctionExpression(reassigned) = &assign.right else {
        return false;
    };
    let Some(reassigned_body) = reassigned.body.as_ref() else {
        return false;
    };
    if reassigned_body.statements.len() != 1 {
        return false;
    }
    let Statement::ReturnStatement(ret) = &reassigned_body.statements[0] else {
        return false;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return false;
    };
    if ret_id.name.as_str() != local_name {
        return false;
    }

    // Statement 3: return <local>;
    let Statement::ReturnStatement(ret) = &body.statements[2] else {
        return false;
    };
    let Some(Expression::Identifier(ret_id)) = &ret.argument else {
        return false;
    };
    ret_id.name.as_str() == local_name
}

fn assignment_target_name<'a>(target: &oxc_ast::ast::AssignmentTarget<'a>) -> Option<&'a str> {
    use oxc_ast::ast::AssignmentTarget;
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}
