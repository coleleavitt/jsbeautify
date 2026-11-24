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
use oxc_ast::ast::{Class, Declaration, ExportDefaultDeclarationKind, Expression, Function, Program, Statement};
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
    let mut shapes = BTreeMap::new();
    let source = program.source_text;
    let mut add = |name: &str, kind: &str, hash: u64| {
        if locals.contains(name) {
            shapes.insert(name.to_string(), format!("{kind}_{:08x}", fold(hash)));
        }
    };
    for statement in &program.body {
        let declaration = match statement {
            Statement::ExportDeclaration(export) => Some(&export.declaration),
            Statement::ExportDefaultDeclaration(export) => {
                match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        if let Some(id) = &func.id {
                            add(&id.name, "fn", hash_function(source, func));
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        if let Some(id) = &class.id {
                            add(&id.name, "class", hash_class(source, class));
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
                    add(&id.name, "fn", hash_function(source, func));
                }
            }
            Some(Declaration::ClassDeclaration(class)) => {
                if let Some(id) = &class.id {
                    add(&id.name, "class", hash_class(source, class));
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
                    add(&id.name, kind, hasher.hash);
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
        if is_minified(name) {
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
    fn kinds_and_unlisted_locals() {
        let s = shapes("class Ab{}var Cd=1,Ef=function(){};let Gh=2", &["Ab", "Cd", "Ef"]);
        assert!(s["Ab"].starts_with("class_"));
        assert!(s["Cd"].starts_with("val_"));
        assert!(s["Ef"].starts_with("fn_"));
        assert!(!s.contains_key("Gh"));
    }
}
