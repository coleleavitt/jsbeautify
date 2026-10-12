use super::tree::{ReversedSource, write_reversed_sources};
use crate::ast_deobfuscate::bun::Bundler;
use crate::bun_extract::{ModuleType, split_bundle};
use crate::turbopack_extract::detect_turbopack_chunk;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, Class, Expression, Function, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use oxc_syntax::operator::AssignmentOperator;
use oxc_syntax::scope::ScopeFlags;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

pub(super) fn reconstruct_without_sourcemap(bundle_source: &str, output_dir: &Path) -> Result<Vec<String>, String> {
    write_reversed_sources(synthetic_sources(bundle_source), output_dir)
}

fn synthetic_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let turbopack = turbopack_sources(bundle_source);
    if !turbopack.is_empty() {
        return turbopack;
    }

    let lazy_sources = lazy_module_sources(bundle_source);
    if !lazy_sources.is_empty() {
        return lazy_sources;
    }

    let boundary_sources = named_boundary_sources(bundle_source);
    if !boundary_sources.is_empty() {
        return boundary_sources;
    }

    vec![ReversedSource {
        source_path: "synthetic/chunks/bundle.js".to_string(),
        content: bundle_source.to_string(),
    }]
}

/// One file per Turbopack module factory (a one-module chunk registering it),
/// hoisted-module stubs, and `chunk.js`: the chunk with each factory replaced
/// by a comment naming its module.
fn turbopack_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let Some(chunk) = detect_turbopack_chunk(bundle_source) else {
        return Vec::new();
    };
    let root = "synthetic/turbopack";
    let mut out = Vec::new();
    let mut paths = BTreeMap::new();
    let mut skeleton = String::with_capacity(bundle_source.len() / 8);
    let mut last = 0;
    for module in chunk.modules() {
        skeleton.push_str(&bundle_source[last..module.start]);
        let _ = write!(skeleton, "/* turbopack module {} */", module.id().replace("*/", "*\\/"));
        last = module.end;
    }
    skeleton.push_str(&bundle_source[last..]);
    push_source(&mut out, &mut paths, format!("{root}/chunk.js"), skeleton);
    for (name, content) in chunk.module_files(bundle_source) {
        push_source(&mut out, &mut paths, format!("{root}/{name}"), content);
    }
    out
}

/// Splits the lazy `__esm` / `__commonJS` module wrappers that esbuild and Bun
/// emit, under a folder named for the bundler the runtime helpers identify.
fn lazy_module_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let split = split_bundle(bundle_source);
    if split.modules.is_empty() {
        return Vec::new();
    }
    let root = match split.bundler {
        Some(Bundler::Bun) => "synthetic/bun",
        Some(Bundler::Esbuild) => "synthetic/esbuild",
        None => "synthetic/modules",
    };

    let mut out = Vec::with_capacity(split.modules.len() + 2);
    let mut paths = BTreeMap::new();
    push_source(&mut out, &mut paths, format!("{root}/runtime.js"), split.runtime);

    for (module, code) in split.modules {
        let kind = match module.module_type {
            ModuleType::CommonJs => "commonjs",
            ModuleType::Esm => "esm",
            ModuleType::Runtime | ModuleType::Main => "chunks",
        };
        push_source(
            &mut out,
            &mut paths,
            format!("{root}/{kind}/{}.js", safe_component(&module.name)),
            code,
        );
    }

    push_source(&mut out, &mut paths, format!("{root}/entry.js"), split.main);
    out
}

#[derive(Clone, Copy)]
enum BoundaryKind {
    Class,
    Function,
}

/// One named function or class cut from the bundle, already a standalone
/// parseable unit.
struct Boundary {
    name: String,
    kind: BoundaryKind,
    content: String,
}

fn named_boundary_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let mut out = Vec::new();
    let mut paths = BTreeMap::new();
    for boundary in named_boundaries(bundle_source) {
        let dir = match boundary.kind {
            BoundaryKind::Class => "classes",
            BoundaryKind::Function => "functions",
        };
        push_source(
            &mut out,
            &mut paths,
            format!("synthetic/{dir}/{}.js", safe_component(&boundary.name)),
            boundary.content,
        );
    }
    out
}

/// The outermost named functions and classes, in source order, cut at their
/// Oxc spans. An anonymous class is cut with the binding that names it
/// (`var X = class …;`, `X = class …`). A bundle that fails to parse both as
/// a `.cjs` script and as a module has no boundaries.
fn named_boundaries(source: &str) -> Vec<Boundary> {
    crate::stack::run_on_sized_stack(source.len(), || {
        [SourceType::cjs(), SourceType::mjs()]
            .into_iter()
            .find_map(|source_type| {
                let allocator = Allocator::default();
                let parsed = Parser::new(&allocator, source, source_type).parse();
                if parsed.panicked || !parsed.diagnostics.is_empty() {
                    return None;
                }
                let mut finder = BoundaryFinder {
                    source,
                    declaration_kind: None,
                    boundaries: Vec::new(),
                };
                finder.visit_program(&parsed.program);
                Some(finder.boundaries)
            })
    })
    .unwrap_or_default()
}

struct BoundaryFinder<'s> {
    source: &'s str,
    /// The keyword of the variable declaration being walked, when a
    /// declarator of it can stand alone as `<keyword> <declarator>;`.
    declaration_kind: Option<&'static str>,
    boundaries: Vec<Boundary>,
}

impl BoundaryFinder<'_> {
    fn push(&mut self, name: &str, kind: BoundaryKind, content: String) {
        self.boundaries.push(Boundary {
            name: name.to_string(),
            kind,
            content,
        });
    }

    fn text(&self, span: Span) -> &str {
        &self.source[span.start as usize..span.end as usize]
    }
}

fn is_anonymous_class(expr: &Expression<'_>) -> bool {
    matches!(expr.without_parentheses(), Expression::ClassExpression(class) if class.id.is_none())
}

impl<'a> Visit<'a> for BoundaryFinder<'_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        match &func.id {
            Some(id) => {
                let content = self.text(func.span).to_string();
                self.push(&id.name, BoundaryKind::Function, content);
            }
            None => walk::walk_function(self, func, flags),
        }
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        match &class.id {
            Some(id) => {
                let content = self.text(class.span).to_string();
                self.push(&id.name, BoundaryKind::Class, content);
            }
            None => walk::walk_class(self, class),
        }
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        let outer = self.declaration_kind.replace(match decl.kind {
            VariableDeclarationKind::Var => "var",
            VariableDeclarationKind::Let => "let",
            VariableDeclarationKind::Const => "const",
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => {
                self.declaration_kind = None;
                walk::walk_variable_declaration(self, decl);
                return;
            }
        });
        walk::walk_variable_declaration(self, decl);
        self.declaration_kind = outer;
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let (Some(keyword), BindingPattern::BindingIdentifier(id), Some(init)) =
            (self.declaration_kind, &declarator.id, &declarator.init)
            && is_anonymous_class(init)
        {
            let content = format!("{keyword} {};", self.text(declarator.span));
            self.push(&id.name, BoundaryKind::Class, content);
            return;
        }
        let outer = self.declaration_kind.take();
        walk::walk_variable_declarator(self, declarator);
        self.declaration_kind = outer;
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        let name = match &assign.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.name.as_str()),
            AssignmentTarget::StaticMemberExpression(member) => Some(member.property.name.as_str()),
            _ => None,
        };
        if let Some(name) = name
            && assign.operator == AssignmentOperator::Assign
            && is_anonymous_class(&assign.right)
        {
            let content = format!("{};", self.text(assign.span));
            self.push(name, BoundaryKind::Class, content);
            return;
        }
        walk::walk_assignment_expression(self, assign);
    }
}

fn push_source(
    out: &mut Vec<ReversedSource>,
    paths: &mut BTreeMap<String, usize>,
    source_path: String,
    content: String,
) {
    if content.trim().is_empty() {
        return;
    }
    out.push(ReversedSource {
        source_path: unique_path(source_path, paths),
        content,
    });
}

fn unique_path(path: String, paths: &mut BTreeMap<String, usize>) -> String {
    let count = paths.entry(path.clone()).or_default();
    *count += 1;
    if *count == 1 {
        return path;
    }

    let suffix = *count;
    if let Some((stem, ext)) = path.rsplit_once('.') {
        format!("{stem}_{suffix}.{ext}")
    } else {
        format!("{path}_{suffix}")
    }
}

fn safe_component(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim_matches('_');
    if trimmed.is_empty() {
        "anonymous".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(path, content)` of every synthetic source, asserting each one parses
    /// on its own as a classic script.
    fn split(bundle: &str) -> Vec<(String, String)> {
        synthetic_sources(bundle)
            .into_iter()
            .map(|source| {
                let allocator = Allocator::default();
                let parsed = Parser::new(&allocator, &source.content, SourceType::script()).parse();
                assert!(
                    parsed.diagnostics.is_empty(),
                    "{} does not parse alone: {:?}\n{}",
                    source.source_path,
                    parsed.diagnostics,
                    source.content
                );
                (source.source_path, source.content)
            })
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(path, content)| ((*path).to_string(), (*content).to_string()))
            .collect()
    }

    #[test]
    fn anonymous_classes_are_cut_with_their_binding() {
        let bundle = concat!(
            "var m=class extends Error{name=\"A\"},n=1;",
            "let B;B=class extends m{constructor(e){super(e)}};",
            "e.C=class{};",
            "f(class extends m{});"
        );
        assert_eq!(
            split(bundle),
            pairs(&[
                ("synthetic/classes/m.js", "var m=class extends Error{name=\"A\"};"),
                ("synthetic/classes/B.js", "B=class extends m{constructor(e){super(e)}};"),
                ("synthetic/classes/C.js", "e.C=class{};"),
            ])
        );
    }

    #[test]
    fn functions_with_braces_in_their_parameters_are_cut_whole() {
        let bundle = "async function L(e,o={}){return o}function q({a:b,c:d=\"}\"},...r){return b}";
        assert_eq!(
            split(bundle),
            pairs(&[
                ("synthetic/functions/L.js", "async function L(e,o={}){return o}"),
                ("synthetic/functions/q.js", "function q({a:b,c:d=\"}\"},...r){return b}"),
            ])
        );
    }

    #[test]
    fn keywords_inside_strings_comments_and_templates_are_not_boundaries() {
        let bundle = concat!(
            "var s=\"class here \\u2014 function nope(x){\";",
            "/* class Gone {} */",
            "var t=`function also(){} ${s} class Not{}`;",
            "function f(){return s+t}"
        );
        assert_eq!(
            split(bundle),
            pairs(&[("synthetic/functions/f.js", "function f(){return s+t}")])
        );
    }

    #[test]
    fn regex_and_template_braces_do_not_end_a_boundary_early() {
        let bundle = "function f(t){let r=/\\{(?<a>[^}]*)\\}/;return`${ {a:t}.a }}`+r}function*k$(r){yield*r}";
        assert_eq!(
            split(bundle),
            pairs(&[
                (
                    "synthetic/functions/f.js",
                    "function f(t){let r=/\\{(?<a>[^}]*)\\}/;return`${ {a:t}.a }}`+r}"
                ),
                ("synthetic/functions/k$.js", "function*k$(r){yield*r}"),
            ])
        );
    }

    #[test]
    fn nested_boundaries_stay_inside_the_outermost_one() {
        let bundle = "!function(){function outer(){function inner(){}class Local{}}class Top{m(){function no(){}}}}();";
        assert_eq!(
            split(bundle),
            pairs(&[
                (
                    "synthetic/functions/outer.js",
                    "function outer(){function inner(){}class Local{}}"
                ),
                ("synthetic/classes/Top.js", "class Top{m(){function no(){}}}"),
            ])
        );
    }

    #[test]
    fn module_bundles_are_split_and_unparseable_ones_kept_whole() {
        let module = "import a from\"a\";export function g(){return a}export default class extends a{}";
        assert_eq!(
            split(module),
            pairs(&[("synthetic/functions/g.js", "function g(){return a}")])
        );
        let broken = "function a(){ class B {";
        let sources = synthetic_sources(broken);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_path, "synthetic/chunks/bundle.js");
        assert_eq!(sources[0].content, broken);
    }
}
