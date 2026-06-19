use rustc_hash::FxHashMap;

/// Represents a reconstructed Angular template element.
#[derive(Debug, Clone)]
pub enum TemplateNode {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        bindings: Vec<TemplateBinding>,
        children: Vec<Self>,
        self_closing: bool,
    },
    Text(String),
    Interpolation(String),
    Comment(String),
    NgTemplate {
        structural_directive: Option<String>,
        children: Vec<Self>,
    },
    DeferBlock {
        trigger: Option<String>,
        children: Vec<Self>,
    },
    ConditionalBlock {
        condition: String,
        children: Vec<Self>,
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

#[derive(Debug, Clone)]
pub(super) struct PartialElement {
    pub(super) tag: String,
    pub(super) attrs: Vec<(String, String)>,
    pub(super) bindings: Vec<TemplateBinding>,
    pub(super) children: Vec<TemplateNode>,
}

pub(super) type PendingBindings = FxHashMap<usize, Vec<TemplateBinding>>;
