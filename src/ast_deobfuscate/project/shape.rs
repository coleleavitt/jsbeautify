//! Structural fingerprints of a file's exported top-level declarations.
//!
//! Project mode names a minified binding that nothing else names after the
//! shape of its declaration (`fn_1a2b3c4d`): node kinds, operators, literals,
//! property names and unminified identifiers, with minified identifiers
//! erased. A bundler renames minified bindings and chunk files on every
//! build, so names built from either change between two versions of one
//! program; the shape changes only when the code does.

use std::collections::{BTreeMap, HashSet};
use std::hash::BuildHasher;

use oxc_ast::AstKind;
use oxc_ast::ast::{
    BindingIdentifier, Class, Declaration, ExportDefaultDeclarationKind, Expression, Function, ModuleExportName,
    Program, Statement,
};
use oxc_ast_visit::Visit;
use oxc_span::GetSpan;

use crate::ast_deobfuscate::default_export::is_minified;

/// `fn_…`, `class_…` or `val_…` for each of `locals` declared at the top
/// level of `program`, by local name.
#[must_use]
pub fn declaration_shapes<S: BuildHasher>(
    program: &Program<'_>,
    locals: &HashSet<&str, S>,
) -> BTreeMap<String, String> {
    shapes_where(program, true, |name| locals.contains(name))
        .into_iter()
        .map(|(name, _, shape)| (name, shape))
        .collect()
}

/// The shapes of the minified top-level bindings a module declares but does
/// not export, by the source offset of the binding's name in its declaration.
///
/// Project mode gives them these names (Phase 12.4) when no pass found a
/// better one, so they too stay the same across builds. The offset, unlike
/// the name, stays with the binding when an earlier phase renamed it; the
/// caller must use it with an analysis of this same `program`.
#[must_use]
pub fn local_shapes(program: &Program<'_>) -> BTreeMap<u32, String> {
    let exported: HashSet<&str> = program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ExportNamedDeclaration(export) => Some(export),
            _ => None,
        })
        .flat_map(|export| export.specifiers.iter())
        .filter_map(|spec| match &spec.local {
            ModuleExportName::IdentifierReference(local) => Some(local.name.as_str()),
            _ => None,
        })
        .collect();
    let mut shapes: BTreeMap<u32, String> =
        shapes_where(program, false, |name| is_minified(name) && !exported.contains(name))
            .into_iter()
            .map(|(_, offset, shape)| (offset, shape))
            .collect();
    let mut nested = NestedShapes {
        source: program.source_text,
        depth: 0,
        shapes: &mut shapes,
    };
    nested.visit_program(program);
    shapes
}

/// Functions and classes declared with a minified name inside a function:
/// `function f(){ function Wd(){…} const ef = () => …; }`. Their shape is their
/// own code, not their parent's, so an edit to the parent leaves them named.
struct NestedShapes<'s, 'm> {
    source: &'s str,
    /// How many functions enclose the node being visited.
    depth: usize,
    shapes: &'m mut BTreeMap<u32, String>,
}

impl NestedShapes<'_, '_> {
    fn add(&mut self, id: &BindingIdentifier<'_>, kind: &str, hash: u64) {
        if self.depth > 0 && is_minified(&id.name) {
            self.shapes.insert(id.span.start, format!("{kind}_{:08x}", fold(hash)));
        }
    }
}

impl<'a> Visit<'a> for NestedShapes<'_, '_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if func.is_declaration()
            && let Some(id) = &func.id
        {
            self.add(id, "fn", hash_function(self.source, func));
        }
        self.depth += 1;
        oxc_ast_visit::walk::walk_function(self, func, flags);
        self.depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) {
        self.depth += 1;
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.depth -= 1;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if class.is_declaration()
            && let Some(id) = &class.id
        {
            self.add(id, "class", hash_class(self.source, class));
        }
        oxc_ast_visit::walk::walk_class(self, class);
    }

    fn visit_variable_declarator(&mut self, declarator: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let (Some(id), Some(init)) = (declarator.id.get_binding_identifier(), &declarator.init) {
            let kind = match init.without_parentheses() {
                Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => Some("fn"),
                Expression::ClassExpression(_) => Some("class"),
                _ => None,
            };
            if let Some(kind) = kind {
                let mut hasher = ShapeHasher::new(self.source);
                hasher.visit_expression(init);
                self.add(id, kind, hasher.hash);
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, declarator);
    }
}

/// Shapes of the top-level declarations whose name `keep` accepts;
/// `exported` includes `export function` / `export default` declarations.
/// Each entry is the name, the offset of the name in its declaration, and
/// the shape.
fn shapes_where(program: &Program<'_>, exported: bool, keep: impl Fn(&str) -> bool) -> Vec<(String, u32, String)> {
    let mut shapes = Vec::new();
    let source = program.source_text;
    let mut add = |id: &BindingIdentifier<'_>, kind: &str, hash: u64| {
        if keep(&id.name) {
            shapes.push((id.name.to_string(), id.span.start, format!("{kind}_{:08x}", fold(hash))));
        }
    };
    for statement in &program.body {
        let declaration = match statement {
            Statement::ExportDeclaration(export) => exported.then_some(&export.declaration),
            Statement::ExportDefaultDeclaration(_) if !exported => None,
            Statement::ExportDefaultDeclaration(export) => {
                match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        if let Some(id) = &func.id {
                            add(id, "fn", hash_function(source, func));
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        if let Some(id) = &class.id {
                            add(id, "class", hash_class(source, class));
                        }
                    }
                    _ => {}
                }
                None
            }
            _ => statement.as_declaration(),
        };
        match declaration {
            Some(Declaration::FunctionDeclaration(func)) => {
                if let Some(id) = &func.id {
                    add(id, "fn", hash_function(source, func));
                }
            }
            Some(Declaration::ClassDeclaration(class)) => {
                if let Some(id) = &class.id {
                    add(id, "class", hash_class(source, class));
                }
            }
            Some(Declaration::VariableDeclaration(var)) => {
                for declarator in &var.declarations {
                    let (Some(id), Some(init)) = (declarator.id.get_binding_identifier(), &declarator.init) else {
                        continue;
                    };
                    let kind = match init.without_parentheses() {
                        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => "fn",
                        Expression::ClassExpression(_) => "class",
                        _ => "val",
                    };
                    let mut hasher = ShapeHasher::new(source);
                    hasher.visit_expression(init);
                    add(id, kind, hasher.hash);
                }
            }
            _ => {}
        }
    }
    shapes
}

fn hash_function(source: &str, func: &Function<'_>) -> u64 {
    let mut hasher = ShapeHasher::new(source);
    hasher.visit_function(func, oxc_syntax::scope::ScopeFlags::Function);
    hasher.hash
}

fn hash_class(source: &str, class: &Class<'_>) -> u64 {
    let mut hasher = ShapeHasher::new(source);
    hasher.visit_class(class);
    hasher.hash
}

/// A name this module made (`fn_1a2b3c4d`, `val_…$2`): it changes whenever
/// that binding's code does, so a shape that read it would change with it.
fn is_shape_name(name: &str) -> bool {
    let base = name.split('$').next().unwrap_or(name);
    ["fn_", "class_", "val_"].iter().any(|prefix| {
        base.strip_prefix(prefix)
            .is_some_and(|hex| hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
    })
}

/// 32 bits of a 64-bit hash.
const fn fold(hash: u64) -> u32 {
    #[allow(clippy::cast_possible_truncation)]
    let folded = (hash ^ (hash >> 32)) as u32;
    folded
}

/// FNV-1a over the shape of the visited nodes: a fixed algorithm, so a shape
/// is the same in every run and on every platform.
struct ShapeHasher<'s> {
    source: &'s str,
    hash: u64,
}

impl<'s> ShapeHasher<'s> {
    const fn new(source: &'s str) -> Self {
        Self {
            source,
            hash: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.hash ^= u64::from(byte);
            self.hash = self.hash.wrapping_mul(0x0100_0000_01b3);
        }
        // A separator, so `ab` + `c` differs from `a` + `bc`.
        self.hash ^= 0xff;
        self.hash = self.hash.wrapping_mul(0x0100_0000_01b3);
    }

    fn name(&mut self, name: &str) {
        if is_minified(name) || is_shape_name(name) {
            self.bytes(b"_");
        } else {
            self.bytes(name.as_bytes());
        }
    }
}

impl<'a> Visit<'a> for ShapeHasher<'_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.bytes(&[kind.ty() as u8]);
        match kind {
            AstKind::IdentifierName(it) => self.bytes(it.name.as_bytes()),
            AstKind::PrivateIdentifier(it) => self.bytes(it.name.as_bytes()),
            AstKind::IdentifierReference(it) => self.name(&it.name),
            AstKind::BindingIdentifier(it) => self.name(&it.name),
            AstKind::BinaryExpression(it) => self.bytes(it.operator.as_str().as_bytes()),
            AstKind::LogicalExpression(it) => self.bytes(it.operator.as_str().as_bytes()),
            AstKind::UnaryExpression(it) => self.bytes(it.operator.as_str().as_bytes()),
            AstKind::UpdateExpression(it) => self.bytes(it.operator.as_str().as_bytes()),
            AstKind::AssignmentExpression(it) => self.bytes(it.operator.as_str().as_bytes()),
            AstKind::StringLiteral(_)
            | AstKind::NumericLiteral(_)
            | AstKind::BigIntLiteral(_)
            | AstKind::BooleanLiteral(_)
            | AstKind::RegExpLiteral(_)
            | AstKind::TemplateElement(_) => {
                let span = kind.span();
                let text = self
                    .source
                    .get(span.start as usize..span.end as usize)
                    .unwrap_or_default();
                self.bytes(text.as_bytes());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::*;

    fn shapes(code: &str, locals: &[&str]) -> BTreeMap<String, String> {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        declaration_shapes(&program, &locals.iter().copied().collect::<rustc_hash::FxHashSet<_>>())
    }

    #[test]
    fn renamed_minified_bindings_keep_their_shape() {
        let a = shapes("function Xa(b){return Yc(b).settings}export{Xa as q}", &["Xa"]);
        let b = shapes("function Mt(e){return Pq(e).settings}export{Mt as z}", &["Mt"]);
        assert_eq!(a["Xa"], b["Mt"]);
        assert!(a["Xa"].starts_with("fn_"), "{a:?}");
    }

    #[test]
    fn changed_code_changes_the_shape() {
        let a = shapes("const Xa=(b)=>b.settings", &["Xa"]);
        let b = shapes("const Xa=(b)=>b.policies", &["Xa"]);
        let c = shapes("const Xa=(b)=>b+1", &["Xa"]);
        let d = shapes("const Xa=(b)=>b-1", &["Xa"]);
        assert_ne!(a["Xa"], b["Xa"]);
        assert_ne!(c["Xa"], d["Xa"]);
    }

    #[test]
    fn local_shapes_skip_exports_and_long_names() {
        let allocator = Allocator::default();
        let code = "function Ab(){return 1}function Cd(){return 2}export function Ef(){}export{Cd as x};function readable(){}var Gh=3";
        let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        let offsets: Vec<u32> = local_shapes(&program).into_keys().collect();
        let at = |name: &str| u32::try_from(code.find(name).expect("in code")).expect("small");
        assert_eq!(offsets, [at("Ab"), at("Gh")]);
    }

    #[test]
    fn nested_functions_have_their_own_shape() {
        let shapes_of = |code: &str| {
            let allocator = Allocator::default();
            let program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
            let at = |name: &str| u32::try_from(code.find(name).expect("in code")).expect("small");
            let shapes = local_shapes(&program);
            (
                shapes.get(&at("Wd")).cloned(),
                shapes.get(&at("ef")).cloned(),
                shapes.contains_key(&at("Qq")),
            )
        };
        // The parent's code differs; the nested functions' does not.
        let a = shapes_of("function Ab(){function Wd(x){return x.mode}const ef=y=>y.id;let Qq=1;return Wd}");
        let b =
            shapes_of("function Cd(){console.log(1);function Wd(z){return z.mode}const ef=w=>w.id;let Qq=2;return Wd}");
        assert!(a.0.as_deref().is_some_and(|s| s.starts_with("fn_")), "{a:?}");
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
        assert!(!a.2, "nested values keep their slot names");
    }

    #[test]
    fn kinds_and_unlisted_locals() {
        let s = shapes("class Ab{}var Cd=1,Ef=function(){};let Gh=2", &["Ab", "Cd", "Ef"]);
        assert!(s["Ab"].starts_with("class_"));
        assert!(s["Cd"].starts_with("val_"));
        assert!(s["Ef"].starts_with("fn_"));
        assert!(!s.contains_key("Gh"));
    }
}
