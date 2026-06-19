use super::emit::{emit_node, parse_attr_pairs};
use super::patterns::{
    ADVANCE_RE, ATTRIBUTE_RE, ELEMENT_END_RE, ELEMENT_START_RE, ELEMENT_VOID_RE, LISTENER_RE, PIPE_RE, PROPERTY_RE,
    TEXT_INTERPOLATE_RE, TEXT_RE,
};
use super::types::{PartialElement, PendingBindings, TemplateBinding, TemplateNode};

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
    pending_bindings: PendingBindings,
    /// Current slot index (tracked via ɵɵadvance)
    current_slot: usize,
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
            pending_bindings: PendingBindings::default(),
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
            if literal_text.is_empty() {
                self.push_node(TemplateNode::Interpolation("{{...}}".to_string()));
            } else {
                self.push_node(TemplateNode::Text(literal_text));
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
