//! Approximate template reconstruction from Ivy instruction streams.

mod extract;
mod render;
mod roles;

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{ArrayExpressionElement, AssignmentTarget, Expression};
use oxc_parser::Parser;
use oxc_semantic::Semantic;
use oxc_span::SourceType;
use rustc_hash::FxHashMap;

use super::naming::object_property;
use crate::ast_deobfuscate::semantic;
use extract::{Extractor, Instructions, TemplateFn, as_function};
use render::{Bundle, ConstAttrs, Renderer};

struct Component<'a> {
    /// Byte offset of the `template` property; the comment goes above it.
    insert_at: u32,
    template: TemplateFn<'a>,
    consts: Vec<ConstAttrs<'a>>,
}

/// Inserts a `// ` comment block with the reconstructed HTML above the
/// `template` property of every `ɵcmp` definition in `code`.
///
/// Only comments are added, so runtime behaviour is unchanged; callers still
/// hold the result to the output re-parse gate. Returns the annotated code and
/// the number of templates annotated.
#[must_use]
pub fn annotate_angular_templates(code: &str, source_type: SourceType) -> (String, usize) {
    if !code.contains("ɵcmp") && !code.contains("\\u0275cmp") {
        return (code.to_string(), 0);
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return (code.to_string(), 0);
    }
    let program = parsed.program;
    let semantic = semantic::build_with(&program, |builder| builder.with_build_nodes(true)).semantic;
    let mut inserts = decompile(code, &semantic);
    if inserts.is_empty() {
        return (code.to_string(), 0);
    }
    inserts.sort_by_key(|(at, _)| *at);
    let count = inserts.len();
    let mut out = String::with_capacity(code.len() + inserts.iter().map(|(_, t)| t.len()).sum::<usize>());
    let mut cursor = 0;
    for (at, lines) in inserts {
        let at = at as usize;
        out.push_str(&code[cursor..at]);
        let indent = indent_at(code, at);
        out.push_str("// Angular template (decompiled):\n");
        for line in lines.lines() {
            out.push_str(indent);
            out.push_str("// ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(indent);
        cursor = at;
    }
    out.push_str(&code[cursor..]);
    (out, count)
}

fn indent_at(code: &str, at: usize) -> &str {
    let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
    let prefix = &code[line_start..at];
    if prefix.chars().all(char::is_whitespace) {
        prefix
    } else {
        ""
    }
}

fn decompile<'a>(code: &str, semantic: &Semantic<'a>) -> Vec<(u32, String)> {
    let components = find_components(semantic);
    if components.is_empty() {
        return Vec::new();
    }

    let mut extractor = Extractor::new(semantic);
    let mut views: FxHashMap<u32, Instructions<'a>> = FxHashMap::default();
    let mut queue: Vec<TemplateFn<'a>> = components.iter().map(|c| c.template).collect();
    while let Some(func) = queue.pop() {
        if views.contains_key(&func.key()) {
            continue;
        }
        let instructions = extractor.extract(func);
        for call in &instructions.create {
            for index in [1, 8] {
                if let Some(embedded) = call.arg(index).and_then(|e| as_function(e, semantic)) {
                    queue.push(embedded);
                }
            }
        }
        views.insert(func.key(), instructions);
    }

    let mut keys: Vec<u32> = views.keys().copied().collect();
    keys.sort_unstable();
    let ordered: Vec<Instructions<'a>> = keys.iter().filter_map(|k| views.remove(k)).collect();
    let roles = roles::infer_roles(&extractor.callees, &ordered);
    let views: FxHashMap<u32, Instructions<'a>> = keys.into_iter().zip(ordered).collect();

    let bundle = Bundle {
        code,
        semantic,
        roles: &roles,
        views: &views,
    };
    components
        .into_iter()
        .filter_map(|component| {
            let mut renderer = Renderer::new(&bundle, component.consts, component.template);
            renderer.render(component.template, 0);
            let text: Vec<String> = renderer.lines.iter().map(|line| sanitize(line)).collect();
            (!text.is_empty()).then(|| (component.insert_at, text.join("\n")))
        })
        .collect()
}

/// Keeps every comment line on one physical line.
fn sanitize(line: &str) -> String {
    line.chars()
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn find_components<'a>(semantic: &Semantic<'a>) -> Vec<Component<'a>> {
    let mut components = Vec::new();
    for node in semantic.nodes().iter() {
        let value = match node.kind() {
            AstKind::PropertyDefinition(prop)
                if prop.r#static && prop.key.static_name().is_some_and(|k| k == "ɵcmp") =>
            {
                prop.value.as_ref()
            }
            AstKind::AssignmentExpression(assign) => match &assign.left {
                AssignmentTarget::StaticMemberExpression(member) if member.property.name == "ɵcmp" => {
                    Some(&assign.right)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(component) = value.and_then(|v| component(v, semantic)) {
            components.push(component);
        }
    }
    components
}

fn component<'a>(value: &'a Expression<'a>, semantic: &Semantic<'a>) -> Option<Component<'a>> {
    let Expression::CallExpression(call) = value.without_parentheses() else {
        return None;
    };
    let Some(Expression::ObjectExpression(def)) = call.arguments.first().and_then(|a| a.as_expression()) else {
        return None;
    };
    let template_property = def.properties.iter().find_map(|p| match p {
        oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) if p.key.static_name().is_some_and(|k| k == "template") => {
            Some(p)
        }
        _ => None,
    })?;
    let template = as_function(&template_property.value, semantic)?;
    let consts = match object_property(def, "consts") {
        Some(Expression::ArrayExpression(list)) => list
            .elements
            .iter()
            .map(|e| match e {
                ArrayExpressionElement::ArrayExpression(attrs) => attrs.elements.as_slice(),
                _ => &[],
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Component {
        insert_at: template_property.span.start,
        template,
        consts,
    })
}
