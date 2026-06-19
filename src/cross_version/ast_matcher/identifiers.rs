use oxc_ast::ast::{BindingIdentifier, IdentifierReference};
use oxc_ast_visit::Visit;

use super::IdentifierInfo;

pub(super) struct IdentifierCollector {
    pub(super) identifiers: Vec<IdentifierInfo>,
    order_index: usize,
}

impl IdentifierCollector {
    pub(super) const fn new() -> Self {
        Self {
            identifiers: Vec::new(),
            order_index: 0,
        }
    }

    fn is_reserved(name: &str) -> bool {
        const RESERVED: &[&str] = &[
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "null",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "yield",
            "implements",
            "interface",
            "package",
            "private",
            "protected",
            "public",
            "arguments",
            "eval",
            "undefined",
            "NaN",
            "Infinity",
            "Object",
            "Array",
            "String",
            "Number",
            "Boolean",
            "Function",
            "Symbol",
            "Error",
            "Promise",
            "Map",
            "Set",
            "WeakMap",
            "WeakSet",
            "JSON",
            "Math",
            "console",
            "process",
            "require",
            "module",
            "exports",
            "global",
            "globalThis",
            "window",
            "document",
        ];
        RESERVED.contains(&name)
    }
}

impl<'a> Visit<'a> for IdentifierCollector {
    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if !Self::is_reserved(ident.name.as_str()) {
            self.identifiers.push(IdentifierInfo {
                name: ident.name.to_string(),
                start: ident.span.start,
                end: ident.span.end,
                order_index: self.order_index,
            });
            self.order_index += 1;
        }
    }

    fn visit_binding_identifier(&mut self, ident: &BindingIdentifier<'a>) {
        if !Self::is_reserved(ident.name.as_str()) {
            self.identifiers.push(IdentifierInfo {
                name: ident.name.to_string(),
                start: ident.span.start,
                end: ident.span.end,
                order_index: self.order_index,
            });
            self.order_index += 1;
        }
    }
}
