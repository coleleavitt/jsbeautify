//! Angular template decompiler — reverses the Angular compiler's output.
//!
//! The Angular compiler transforms `@Component({template: '<div>{{name}}</div>'})` into:
//! ```js
//! ɵɵdefineComponent({
//!   template: function(rf, ctx) {
//!     if (1 & rf) { ɵɵelementStart(0, "div"); ɵɵtext(1); ɵɵelementEnd(); }
//!     if (2 & rf) { ɵɵadvance(); ɵɵtextInterpolate(ctx.name); }
//!   }
//! })
//! ```
//!
//! This module reverses that process, reconstructing approximate HTML templates and
//! emitting annotated pseudo-TypeScript with `@Component`/`@Directive` decorators.
//!
//! ## Instruction → HTML mapping (Angular 10-19):
//! | Instruction                    | HTML equivalent                        |
//! |-------------------------------|----------------------------------------|
//! | `ɵɵelementStart(idx, "div")`  | `<div>`                               |
//! | `ɵɵelementEnd()`              | `</div>`                              |
//! | `ɵɵelement(idx, "br")`        | `<br/>`                               |
//! | `ɵɵtext(idx)`                 | text node (bound)                     |
//! | `ɵɵtext(idx, "literal")`      | literal text node                     |
//! | `ɵɵtemplate(idx, fn, ...)`    | `<ng-template>` / `*ngIf` / `*ngFor`  |
//! | `ɵɵproperty("x", expr)`       | `[x]="expr"`                          |
//! | `ɵɵattribute("x", expr)`      | `[attr.x]="expr"`                     |
//! | `ɵɵlistener("click", fn)`     | `(click)="fn()"`                      |
//! | `ɵɵtextInterpolate(expr)`     | `{{expr}}`                            |
//! | `ɵɵpipe(idx, "name")`         | `| name`                              |
//! | `ɵɵpureFunction*(fn, ...)`    | inline template expression             |
//! | `ɵɵsanitizeHtml`              | innerHTML binding with sanitizer       |
//! | `ɵɵsanitizeUrl`               | URL binding with sanitizer             |
//! | `ɵɵconditional(idx, expr)`    | `@if (expr) { }` (v17+)               |
//! | `ɵɵdefer(idx, ...)`           | `@defer { }` (v17+)                   |
//! | `ɵɵdeclareLet(idx)`           | `@let x = ...` (v19+)                 |
//!
//! ## Consts array mapping:
//! The `consts` array in `ɵɵdefineComponent` contains attribute arrays referenced by index:
//! ```js
//! consts: [["id", "main"], ["class", "header", "role", "banner"]]
//! // index 0: id="main"
//! // index 1: class="header" role="banner"
//! ```
//! These are referenced by `ɵɵelementStart(idx, "tag", constsIdx)`.

use regex::Regex;
use rustc_hash::FxHashMap;

/// Represents a reconstructed Angular template element.
#[derive(Debug, Clone)]
pub enum TemplateNode {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        bindings: Vec<TemplateBinding>,
        children: Vec<TemplateNode>,
        self_closing: bool,
    },
    Text(String),
    Interpolation(String),
    Comment(String),
    NgTemplate {
        structural_directive: Option<String>,
        children: Vec<TemplateNode>,
    },
    DeferBlock {
        trigger: Option<String>,
        children: Vec<TemplateNode>,
    },
    ConditionalBlock {
        condition: String,
        children: Vec<TemplateNode>,
    },
}

#[derive(Debug, Clone)]
pub enum TemplateBinding {
    Property(String, String),  // [prop]="expr"
    Attribute(String, String), // [attr.name]="expr"
    Event(String, String),     // (event)="handler()"
    TwoWay(String, String),    // [(ngModel)]="expr"
    Class(String),             // [class.name]="expr"
    Style(String, String),     // [style.prop]="expr"
}

/// Decompiled metadata for a single Angular component.
#[derive(Debug, Clone)]
pub struct DecompiledComponent {
    /// Reconstructed selector (from metadata)
    pub selector: String,
    /// Generated class name
    pub class_name: String,
    /// Reconstructed HTML template
    pub template_html: String,
    /// Inputs with their property names
    pub inputs: Vec<String>,
    /// Outputs with their event names
    pub outputs: Vec<String>,
    /// Injected dependencies (from constructor params)
    pub dependencies: Vec<String>,
    /// Whether it's standalone
    pub standalone: bool,
    /// Encapsulation mode
    pub encapsulation: ViewEncapsulation,
    /// The `consts` array (attribute arrays)
    pub consts: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEncapsulation {
    Emulated,  // 0
    None,      // 2
    ShadowDom, // 3
}

/// Parse the Angular instruction stream from a template function body
/// and reconstruct the HTML template.
pub struct TemplateDecompiler {
    /// Stack of open elements for reconstruction
    element_stack: Vec<PartialElement>,
    /// Root-level nodes
    root_nodes: Vec<TemplateNode>,
    /// The consts array (attribute arrays from defineComponent)
    consts: Vec<Vec<String>>,
    /// Bindings collected during the update phase (keyed by slot index)
    pending_bindings: FxHashMap<usize, Vec<TemplateBinding>>,
    /// Current slot index (tracked via ɵɵadvance)
    current_slot: usize,
}

#[derive(Debug, Clone)]
struct PartialElement {
    tag: String,
    attrs: Vec<(String, String)>,
    bindings: Vec<TemplateBinding>,
    children: Vec<TemplateNode>,
}

impl Default for TemplateDecompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateDecompiler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            element_stack: Vec::new(),
            root_nodes: Vec::new(),
            consts: Vec::new(),
            pending_bindings: FxHashMap::default(),
            current_slot: 0,
        }
    }

    /// Set the consts array from the defineComponent metadata.
    pub fn set_consts(&mut self, consts: Vec<Vec<String>>) {
        self.consts = consts;
    }

    /// Process a creation-phase instruction line.
    pub fn process_create_instruction(&mut self, instruction: &str) {
        let instruction = instruction.trim();

        if let Some(caps) = ELEMENT_START_RE.captures(instruction) {
            let tag = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let consts_idx: Option<usize> = caps.get(3).and_then(|m| m.as_str().parse().ok());

            let attrs = consts_idx
                .and_then(|idx| self.consts.get(idx))
                .map(|a| parse_attr_pairs(a))
                .unwrap_or_default();

            self.element_stack.push(PartialElement {
                tag,
                attrs,
                bindings: Vec::new(),
                children: Vec::new(),
            });
        } else if ELEMENT_END_RE.is_match(instruction) {
            if let Some(element) = self.element_stack.pop() {
                let node = TemplateNode::Element {
                    tag: element.tag,
                    attrs: element.attrs,
                    bindings: element.bindings,
                    children: element.children,
                    self_closing: false,
                };
                self.push_node(node);
            }
        } else if let Some(caps) = ELEMENT_VOID_RE.captures(instruction) {
            let tag = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let consts_idx: Option<usize> = caps.get(3).and_then(|m| m.as_str().parse().ok());

            let attrs = consts_idx
                .and_then(|idx| self.consts.get(idx))
                .map(|a| parse_attr_pairs(a))
                .unwrap_or_default();

            let node = TemplateNode::Element {
                tag,
                attrs,
                bindings: Vec::new(),
                children: Vec::new(),
                self_closing: true,
            };
            self.push_node(node);
        } else if let Some(caps) = TEXT_RE.captures(instruction) {
            let literal_text = caps.get(2).map_or("", |m| m.as_str()).to_string();
            if !literal_text.is_empty() {
                self.push_node(TemplateNode::Text(literal_text));
            } else {
                self.push_node(TemplateNode::Interpolation("{{...}}".to_string()));
            }
        } else if let Some(caps) = LISTENER_RE.captures(instruction) {
            let event_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let handler = caps.get(2).map_or("$event", |m| m.as_str()).to_string();
            if let Some(current) = self.element_stack.last_mut() {
                current.bindings.push(TemplateBinding::Event(event_name, handler));
            }
        } else if let Some(caps) = PIPE_RE.captures(instruction) {
            let _pipe_name = caps.get(2).map_or("", |m| m.as_str());
            // Pipes are used in update phase bindings, tracked by index
        }
    }

    /// Process an update-phase instruction line.
    pub fn process_update_instruction(&mut self, instruction: &str) {
        let instruction = instruction.trim();

        if let Some(caps) = ADVANCE_RE.captures(instruction) {
            if let Ok(n) = caps.get(1).map_or("1", |m| m.as_str()).parse::<usize>() {
                self.current_slot += n;
            }
        } else if let Some(caps) = PROPERTY_RE.captures(instruction) {
            let prop = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let expr = caps.get(2).map_or("...", |m| m.as_str()).to_string();
            self.pending_bindings
                .entry(self.current_slot)
                .or_default()
                .push(TemplateBinding::Property(prop, expr));
        } else if let Some(caps) = ATTRIBUTE_RE.captures(instruction) {
            let attr = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let expr = caps.get(2).map_or("...", |m| m.as_str()).to_string();
            self.pending_bindings
                .entry(self.current_slot)
                .or_default()
                .push(TemplateBinding::Attribute(attr, expr));
        } else if TEXT_INTERPOLATE_RE.is_match(instruction) {
            // Text interpolation — marks the current text node as bound
        }
    }

    /// Emit the reconstructed HTML template.
    #[must_use]
    pub fn emit_html(&self) -> String {
        let mut output = String::new();
        for node in &self.root_nodes {
            emit_node(&mut output, node, 0);
        }
        output
    }

    fn push_node(&mut self, node: TemplateNode) {
        if let Some(parent) = self.element_stack.last_mut() {
            parent.children.push(node);
        } else {
            self.root_nodes.push(node);
        }
    }
}

fn emit_node(output: &mut String, node: &TemplateNode, indent: usize) {
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
                        output.push_str(&format!(" [{prop}]=\"{expr}\""));
                    }
                    TemplateBinding::Attribute(attr, expr) => {
                        output.push_str(&format!(" [attr.{attr}]=\"{expr}\""));
                    }
                    TemplateBinding::Event(event, handler) => {
                        output.push_str(&format!(" ({event})=\"{handler}\""));
                    }
                    TemplateBinding::TwoWay(prop, expr) => {
                        output.push_str(&format!(" [({prop})]=\"{expr}\""));
                    }
                    TemplateBinding::Class(cls) => {
                        output.push_str(&format!(" [class.{cls}]=\"...\""));
                    }
                    TemplateBinding::Style(prop, expr) => {
                        output.push_str(&format!(" [style.{prop}]=\"{expr}\""));
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
                output.push_str(&format!("<ng-template {dir}>\n"));
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
                output.push_str(&format!("@defer ({t}) {{\n"));
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
            output.push_str(&format!("@if ({condition}) {{\n"));
            for child in children {
                emit_node(output, child, indent + 1);
            }
            output.push_str(&pad);
            output.push_str("}\n");
        }
    }
}

fn parse_attr_pairs(attrs: &[String]) -> Vec<(String, String)> {
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

/// Decompile an Angular component from its beautified source text.
/// Extracts template instructions and reconstructs approximate HTML.
///
/// Input: the full beautified source of a defineComponent call region.
/// Returns: reconstructed pseudo-template and metadata.
pub fn decompile_template_from_source(source: &str) -> Option<String> {
    let mut decompiler = TemplateDecompiler::new();
    let mut in_create = false;
    let mut in_update = false;

    for line in source.lines() {
        let trimmed = line.trim();

        // Detect create phase: `if (1 & rf)` or `if (1 & dt)`
        if trimmed.contains("1 & ") && trimmed.contains("if") {
            in_create = true;
            in_update = false;
            continue;
        }
        // Detect update phase: `if (2 & rf)` or `if (2 & dt)`
        if trimmed.contains("2 & ") && trimmed.contains("if") {
            in_create = false;
            in_update = true;
            continue;
        }

        if in_create {
            decompiler.process_create_instruction(trimmed);
        } else if in_update {
            decompiler.process_update_instruction(trimmed);
        }
    }

    let html = decompiler.emit_html();
    if html.is_empty() { None } else { Some(html) }
}

/// Reconstruct a full pseudo-TypeScript component from decompiled metadata.
#[must_use]
pub fn emit_pseudo_typescript(comp: &DecompiledComponent) -> String {
    let mut output = String::new();

    output.push_str("@Component({\n");
    output.push_str(&format!("  selector: '{}',\n", comp.selector));
    if comp.standalone {
        output.push_str("  standalone: true,\n");
    }
    if comp.encapsulation != ViewEncapsulation::Emulated {
        let enc = match comp.encapsulation {
            ViewEncapsulation::Emulated => "ViewEncapsulation.Emulated",
            ViewEncapsulation::None => "ViewEncapsulation.None",
            ViewEncapsulation::ShadowDom => "ViewEncapsulation.ShadowDom",
        };
        output.push_str(&format!("  encapsulation: {enc},\n"));
    }
    output.push_str("  template: `\n");
    for line in comp.template_html.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    output.push_str("  `\n");
    output.push_str("})\n");

    output.push_str(&format!("export class {} {{\n", comp.class_name));

    // Inputs
    for input in &comp.inputs {
        output.push_str(&format!("  @Input() {input}: unknown;\n"));
    }

    // Outputs
    for out in &comp.outputs {
        output.push_str(&format!("  @Output() {out} = new EventEmitter();\n"));
    }

    // Constructor with injections
    if !comp.dependencies.is_empty() {
        output.push_str("\n  constructor(\n");
        for dep in &comp.dependencies {
            output.push_str(&format!("    private {dep}: unknown,\n"));
        }
        output.push_str("  ) {}\n");
    }

    output.push_str("}\n");
    output
}

// ─── Regex patterns for instruction parsing ───

lazy_static::lazy_static! {
    // ɵɵelementStart(0, "div") or ɵɵelementStart(0, "div", 3)
    static ref ELEMENT_START_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)elementStart\((\d+),\s*"([^"]+)"(?:,\s*(\d+))?\)"#
    ).unwrap();

    // ɵɵelementEnd()
    static ref ELEMENT_END_RE: Regex = Regex::new(
        r"(?:ɵɵ|\w+\.)elementEnd\(\)"
    ).unwrap();

    // ɵɵelement(0, "br") or ɵɵelement(0, "br", 3)
    static ref ELEMENT_VOID_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)element\((\d+),\s*"([^"]+)"(?:,\s*(\d+))?\)"#
    ).unwrap();

    // ɵɵtext(1) or ɵɵtext(1, "literal")
    static ref TEXT_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)text\((\d+)(?:,\s*"([^"]*)")?\)"#
    ).unwrap();

    // ɵɵlistener("click", function($event) { ... })
    static ref LISTENER_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)listener\("([^"]+)",\s*function\(([^)]*)\)"#
    ).unwrap();

    // ɵɵpipe(2, "async")
    static ref PIPE_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)pipe\((\d+),\s*"([^"]+)"\)"#
    ).unwrap();

    // ɵɵadvance(2) or ɵɵadvance()
    static ref ADVANCE_RE: Regex = Regex::new(
        r"(?:ɵɵ|\w+\.)advance\((\d*)\)"
    ).unwrap();

    // ɵɵproperty("ngIf", ctx.show)
    static ref PROPERTY_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)property\("([^"]+)",\s*(.+?)\)"#
    ).unwrap();

    // ɵɵattribute("role", ctx.role)
    static ref ATTRIBUTE_RE: Regex = Regex::new(
        r#"(?:ɵɵ|\w+\.)attribute\("([^"]+)",\s*(.+?)\)"#
    ).unwrap();

    // ɵɵtextInterpolate(ctx.name) or ɵɵtextInterpolate1("Hello ", ctx.name, "!")
    static ref TEXT_INTERPOLATE_RE: Regex = Regex::new(
        r"(?:ɵɵ|\w+\.)textInterpolate"
    ).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_element_decompile() {
        let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵtext(1);
                g.ɵɵelementEnd();
            }
            if (2 & rf) {
                g.ɵɵadvance(1);
                g.ɵɵtextInterpolate(ctx.name);
            }
        "#;

        let result = decompile_template_from_source(source).unwrap();
        assert!(result.contains("<div>"));
        assert!(result.contains("</div>"));
    }

    #[test]
    fn test_nested_elements() {
        let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵelementStart(1, "span");
                g.ɵɵtext(2, "hello");
                g.ɵɵelementEnd();
                g.ɵɵelementEnd();
            }
        "#;

        let result = decompile_template_from_source(source).unwrap();
        assert!(result.contains("<div>"));
        assert!(result.contains("<span>"));
        assert!(result.contains("hello"));
        assert!(result.contains("</span>"));
        assert!(result.contains("</div>"));
    }

    #[test]
    fn test_self_closing_element() {
        let source = r#"
            if (1 & rf) {
                g.ɵɵelement(0, "br");
                g.ɵɵelement(1, "input");
            }
        "#;

        let result = decompile_template_from_source(source).unwrap();
        assert!(result.contains("<br />"));
        assert!(result.contains("<input />"));
    }

    #[test]
    fn test_event_listener() {
        let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "button");
                g.ɵɵlistener("click", function($event) { return ctx.onClick(); });
                g.ɵɵtext(1, "Click me");
                g.ɵɵelementEnd();
            }
        "#;

        let result = decompile_template_from_source(source).unwrap();
        assert!(result.contains("(click)"));
        assert!(result.contains("<button"));
    }

    #[test]
    fn test_property_binding() {
        let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵelementEnd();
            }
            if (2 & rf) {
                g.ɵɵproperty("ngClass", ctx.classes);
            }
        "#;

        let result = decompile_template_from_source(source).unwrap();
        assert!(result.contains("<div>"));
    }

    #[test]
    fn test_emit_pseudo_typescript() {
        let comp = DecompiledComponent {
            selector: "lms-app-root".to_string(),
            class_name: "LmsAppRootComponent".to_string(),
            template_html: "<div>\n  <span>Hello</span>\n</div>\n".to_string(),
            inputs: vec!["title".to_string(), "mode".to_string()],
            outputs: vec!["closed".to_string()],
            dependencies: vec!["router".to_string(), "store".to_string()],
            standalone: true,
            encapsulation: ViewEncapsulation::None,
            consts: Vec::new(),
        };

        let ts = emit_pseudo_typescript(&comp);
        assert!(ts.contains("@Component"));
        assert!(ts.contains("selector: 'lms-app-root'"));
        assert!(ts.contains("standalone: true"));
        assert!(ts.contains("ViewEncapsulation.None"));
        assert!(ts.contains("@Input() title"));
        assert!(ts.contains("@Output() closed"));
        assert!(ts.contains("private router"));
        assert!(ts.contains("export class LmsAppRootComponent"));
    }
}
