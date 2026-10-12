//! Names for the minified locals behind a module's `default` export.
//!
//! Each export recovery that reads a module's export list (esbuild/Bun
//! `__export`, Phase 0.95; webpack `__webpack_require__.d`, Phase 0.85;
//! Turbopack `esmExport`, Phase 0.86; native ES `export default`, Phase 0.95)
//! hands the local behind its `default` key to [`default_export_names`], which
//! names it from, in order of trust:
//!
//! 1. the module's own identity when it is a path: a webpack/Turbopack string
//!    module id or an esbuild `// path` module comment, named like esbuild names
//!    default exports (`./src/Button.tsx` → `Button_default`);
//! 2. a name the value already carries: a named function or class expression
//!    (`x = function encode() {}`) or a constructor's `this.name = "…"`;
//! 3. `default_<module>`: the numeric module id (`default_396713`), the esbuild
//!    module's ordinal, or `default_export` for an ES module file.
//!
//! Only minified locals ([`is_minified`]) are named; collisions and the
//! shared rename guards are left to the caller's rename planner.

use oxc_ast::ast::{AssignmentTarget, BindingPattern, Class, Expression, Program, VariableDeclarator};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::bun::export_renamer::names::{is_reserved_word, is_valid_identifier, sanitize_identifier};
use crate::ast_deobfuscate::class_name_renamer::constructor_this_name;

/// Minifiers emit one- to three-character bindings; anything longer is
/// treated as an author-chosen name and left alone.
pub fn is_minified(name: &str) -> bool {
    name.chars().count() <= 3
}

/// The local behind a module's `default` export.
pub struct DefaultExport {
    pub local: SymbolId,
    /// A module id, a module path, or `None` for the ES module file itself.
    pub module: Option<String>,
}

/// Desired names for the minified `defaults`, in their order; the first entry
/// for a symbol wins.
pub fn default_export_names(
    program: &Program<'_>,
    scoping: &Scoping,
    defaults: Vec<DefaultExport>,
) -> Vec<(SymbolId, String)> {
    let mut seen = FxHashSet::default();
    let defaults: Vec<DefaultExport> = defaults
        .into_iter()
        .filter(|d| is_minified(scoping.symbol_name(d.local)) && seen.insert(d.local))
        .collect();
    if defaults.is_empty() {
        return Vec::new();
    }
    let mut own = OwnNames {
        scoping,
        wanted: seen,
        names: FxHashMap::default(),
    };
    own.visit_program(program);
    defaults
        .into_iter()
        .map(|d| {
            let path_name = d
                .module
                .as_deref()
                .and_then(path_base)
                .map(|base| format!("{base}_default"));
            let name = path_name
                .or_else(|| own.names.get(&d.local).cloned())
                .unwrap_or_else(|| {
                    let module = d.module.as_deref().unwrap_or("export");
                    sanitize_identifier(&format!("default_{module}"))
                });
            (d.local, name)
        })
        .collect()
}

/// The identifier-safe file name of a module path: `./src/Button.tsx` →
/// `Button`, `[project]/app/page.tsx [app-client] (ecmascript)` → `page`,
/// `./node_modules/left-pad/index.js` → `left_pad`. `None` for numeric ids.
fn path_base(module: &str) -> Option<String> {
    if module.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let path = module.split_whitespace().next()?.split(['?', '#']).next()?;
    let mut segments = path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..");
    let mut base = segments.next_back()?.split('.').next()?;
    if base == "index" {
        base = segments.next_back().unwrap_or(base);
    }
    let base = base.trim_start_matches(['[', '@']).trim_end_matches(']');
    (!base.is_empty()).then(|| sanitize_identifier(base))
}

/// A usable name carried by `expr` itself.
fn own_name(expr: &Expression<'_>) -> Option<String> {
    let name = match expr.without_parentheses() {
        Expression::FunctionExpression(func) => func.id.as_ref().map(|id| id.name.as_str()),
        Expression::ClassExpression(class) => class
            .id
            .as_ref()
            .map(|id| id.name.as_str())
            .filter(|name| !is_minified(name))
            .or_else(|| constructor_this_name(class)),
        _ => None,
    }?;
    let name = if is_valid_identifier(name) {
        name.to_string()
    } else {
        sanitize_identifier(name)
    };
    (!is_minified(&name) && !is_reserved_word(&name)).then_some(name)
}

/// Finds the names the wanted locals' values carry.
struct OwnNames<'s> {
    scoping: &'s Scoping,
    wanted: FxHashSet<SymbolId>,
    names: FxHashMap<SymbolId, String>,
}

impl OwnNames<'_> {
    fn record(&mut self, symbol: Option<SymbolId>, value: &Expression<'_>) {
        if let Some(symbol) = symbol.filter(|s| self.wanted.contains(s))
            && let Some(name) = own_name(value)
        {
            self.names.entry(symbol).or_insert(name);
        }
    }
}

impl<'a> Visit<'a> for OwnNames<'_> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&decl.id, &decl.init) {
            self.record(id.symbol_id.get(), init);
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, assign: &oxc_ast::ast::AssignmentExpression<'a>) {
        if assign.operator.is_assign()
            && let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left
        {
            let symbol = self.scoping.get_reference(id.reference_id()).symbol_id();
            self.record(symbol, &assign.right);
        }
        walk::walk_assignment_expression(self, assign);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if class.is_declaration()
            && let Some(symbol) = class.id.as_ref().and_then(|id| id.symbol_id.get())
            && self.wanted.contains(&symbol)
            && let Some(name) = constructor_this_name(class).filter(|n| is_valid_identifier(n) && !is_minified(n))
        {
            self.names.entry(symbol).or_insert_with(|| name.to_string());
        }
        walk::walk_class(self, class);
    }
}

#[cfg(test)]
mod tests {
    use super::path_base;

    #[test]
    fn path_base_reads_the_file_name() {
        assert_eq!(path_base("./src/Button.tsx").as_deref(), Some("Button"));
        assert_eq!(path_base("src/foo-bar.ts").as_deref(), Some("foo_bar"));
        assert_eq!(
            path_base("./node_modules/left-pad/index.js").as_deref(),
            Some("left_pad")
        );
        assert_eq!(
            path_base("[project]/app/page.tsx [app-client] (ecmascript)").as_deref(),
            Some("page")
        );
        assert_eq!(path_base("396713"), None);
    }
}
