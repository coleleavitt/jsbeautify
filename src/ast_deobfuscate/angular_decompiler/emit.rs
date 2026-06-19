use super::types::{DecompiledComponent, TemplateBinding, TemplateNode, ViewEncapsulation};
use std::fmt::Write as _;

pub(super) fn emit_node(output: &mut String, node: &TemplateNode, indent: usize) {
    let pad = "  ".repeat(indent);
    match node {
        TemplateNode::Element {
            tag,
            attrs,
            bindings,
            children,
            self_closing,
        } => {
            output.push_str(&pad);
            output.push('<');
            output.push_str(tag);

            for (key, value) in attrs {
                output.push(' ');
                output.push_str(key);
                if !value.is_empty() {
                    output.push_str("=\"");
                    output.push_str(value);
                    output.push('"');
                }
            }

            for binding in bindings {
                match binding {
                    TemplateBinding::Property(prop, expr) => {
                        let _ = write!(output, " [{prop}]=\"{expr}\"");
                    }
                    TemplateBinding::Attribute(attr, expr) => {
                        let _ = write!(output, " [attr.{attr}]=\"{expr}\"");
                    }
                    TemplateBinding::Event(event, handler) => {
                        let _ = write!(output, " ({event})=\"{handler}\"");
                    }
                    TemplateBinding::TwoWay(prop, expr) => {
                        let _ = write!(output, " [({prop})]=\"{expr}\"");
                    }
                    TemplateBinding::Class(cls) => {
                        let _ = write!(output, " [class.{cls}]=\"...\"");
                    }
                    TemplateBinding::Style(prop, expr) => {
                        let _ = write!(output, " [style.{prop}]=\"{expr}\"");
                    }
                }
            }

            if *self_closing {
                output.push_str(" />\n");
            } else if children.is_empty() {
                output.push_str("></");
                output.push_str(tag);
                output.push_str(">\n");
            } else {
                output.push_str(">\n");
                for child in children {
                    emit_node(output, child, indent + 1);
                }
                output.push_str(&pad);
                output.push_str("</");
                output.push_str(tag);
                output.push_str(">\n");
            }
        }
        TemplateNode::Text(text) => {
            output.push_str(&pad);
            output.push_str(text);
            output.push('\n');
        }
        TemplateNode::Interpolation(expr) => {
            output.push_str(&pad);
            output.push_str(expr);
            output.push('\n');
        }
        TemplateNode::Comment(text) => {
            output.push_str(&pad);
            output.push_str("<!-- ");
            output.push_str(text);
            output.push_str(" -->\n");
        }
        TemplateNode::NgTemplate {
            structural_directive,
            children,
        } => {
            output.push_str(&pad);
            if let Some(dir) = structural_directive {
                let _ = writeln!(output, "<ng-template {dir}>");
            } else {
                output.push_str("<ng-template>\n");
            }
            for child in children {
                emit_node(output, child, indent + 1);
            }
            output.push_str(&pad);
            output.push_str("</ng-template>\n");
        }
        TemplateNode::DeferBlock { trigger, children } => {
            output.push_str(&pad);
            if let Some(t) = trigger {
                let _ = writeln!(output, "@defer ({t}) {{");
            } else {
                output.push_str("@defer {\n");
            }
            for child in children {
                emit_node(output, child, indent + 1);
            }
            output.push_str(&pad);
            output.push_str("}\n");
        }
        TemplateNode::ConditionalBlock { condition, children } => {
            output.push_str(&pad);
            let _ = writeln!(output, "@if ({condition}) {{");
            for child in children {
                emit_node(output, child, indent + 1);
            }
            output.push_str(&pad);
            output.push_str("}\n");
        }
    }
}

pub(super) fn parse_attr_pairs(attrs: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < attrs.len() {
        let key = &attrs[i];
        if i + 1 < attrs.len() && !attrs[i + 1].is_empty() {
            result.push((key.clone(), attrs[i + 1].clone()));
            i += 2;
        } else {
            result.push((key.clone(), String::new()));
            i += 1;
        }
    }
    result
}

/// Reconstruct a full pseudo-TypeScript component from decompiled metadata.
#[must_use]
pub fn emit_pseudo_typescript(comp: &DecompiledComponent) -> String {
    let mut output = String::new();

    output.push_str("@Component({\n");
    let _ = writeln!(output, "  selector: '{}',", comp.selector);
    if comp.standalone {
        output.push_str("  standalone: true,\n");
    }
    if comp.encapsulation != ViewEncapsulation::Emulated {
        let enc = match comp.encapsulation {
            ViewEncapsulation::Emulated => "ViewEncapsulation.Emulated",
            ViewEncapsulation::None => "ViewEncapsulation.None",
            ViewEncapsulation::ShadowDom => "ViewEncapsulation.ShadowDom",
        };
        let _ = writeln!(output, "  encapsulation: {enc},");
    }
    output.push_str("  template: `\n");
    for line in comp.template_html.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    output.push_str("  `\n");
    output.push_str("})\n");

    let _ = writeln!(output, "export class {} {{", comp.class_name);

    // Inputs
    for input in &comp.inputs {
        let _ = writeln!(output, "  @Input() {input}: unknown;");
    }

    // Outputs
    for out in &comp.outputs {
        let _ = writeln!(output, "  @Output() {out} = new EventEmitter();");
    }

    // Constructor with injections
    if !comp.dependencies.is_empty() {
        output.push_str("\n  constructor(\n");
        for dep in &comp.dependencies {
            let _ = writeln!(output, "    private {dep}: unknown,");
        }
        output.push_str("  ) {}\n");
    }

    output.push_str("}\n");
    output
}
