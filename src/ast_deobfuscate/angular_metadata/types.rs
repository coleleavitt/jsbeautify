use rustc_hash::FxHashMap;

use super::version::AngularVersionRange;

/// Metadata extracted from a single Angular definition call.
#[derive(Debug, Clone)]
pub struct AngularComponentInfo {
    /// The CSS selector (e.g. "lms-app-root", "[myDirective]", "option")
    pub selector: String,
    /// Generated `PascalCase` name (e.g. "`LmsAppRootComponent`")
    pub class_name: String,
    /// The kind of Angular artifact
    pub kind: AngularDefKind,
    /// Inputs declared on this component/directive
    pub inputs: Vec<String>,
    /// Outputs declared on this component/directive
    pub outputs: Vec<String>,
    /// The minified variable name that holds this class (e.g. "Ss", "Fe")
    pub minified_export_name: Option<String>,
    /// The minified class declaration name (e.g. "d", "Le")
    pub minified_class_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngularDefKind {
    Component,
    Directive,
    Pipe,
    Injectable,
    NgModule,
}

impl AngularDefKind {
    pub(super) const fn suffix(self) -> &'static str {
        match self {
            Self::Component => "Component",
            Self::Directive => "Directive",
            Self::Pipe => "Pipe",
            Self::Injectable => "Service",
            Self::NgModule => "Module",
        }
    }
}

#[derive(Debug)]
pub struct AngularExtractionResult {
    pub source: String,
    pub version: Option<AngularVersionRange>,
    pub component_count: usize,
    pub directive_count: usize,
    pub pipe_count: usize,
    pub injectable_count: usize,
    pub module_count: usize,
    pub annotation_count: usize,
    pub definitions: Vec<AngularComponentInfo>,
    pub rename_map: FxHashMap<String, String>,
}
