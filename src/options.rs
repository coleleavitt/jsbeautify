use std::default::Default;
use std::path::PathBuf;

use oxc_span::SourceType;

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Options {
    pub indent_size: usize,
    pub indent_char: String,
    pub indent_with_tabs: bool,
    pub eol: String,
    pub preserve_newlines: bool,
    pub max_preserve_newlines: usize,
    pub space_after_anon_function: bool,
    pub brace_style: BraceStyle,
    pub break_webpack_imports: bool,
    pub add_webpack_module_separators: bool,
    pub extract_large_assets: bool,
    pub asset_size_threshold: usize,
    pub deobfuscate: bool,
    /// Infer readable names for minified locals (loop→i/j/k, catch→error,
    /// require→module name, new Foo→instance, …) instead of `v{n}` slots.
    /// Effective only with `deobfuscate` (the renamer lives in that pipeline).
    pub readable_names: bool,
    pub max_line_length: usize,
    pub generate_source_map: bool,
    pub source_map_file_name: Option<String>,
    pub split_chunks: bool,
    pub chunk_dir: PathBuf,
    pub chunk_map_output: Option<PathBuf>,
    pub extract_modules: bool,
    pub module_dir: PathBuf,
    pub dependency_graph: Option<PathBuf>,
    pub skip_annotations: bool,
    pub parse_goal: ParseGoal,
    /// Promise that nothing outside the input reads its top-level bindings,
    /// so a classic script may have them renamed or removed like module
    /// locals. Module and CommonJS input are already isolated.
    pub isolated: bool,
}

/// How the input is parsed, and therefore who can observe its top-level
/// bindings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParseGoal {
    /// Module when the source contains ESM syntax (`import`, `export`,
    /// `import.meta`, top-level `await`), otherwise classic script.
    #[default]
    Auto,
    /// ES module: top-level bindings are private to the file.
    Module,
    /// Classic script: top-level bindings are shared through the global
    /// object and the global lexical scope.
    Script,
    /// CommonJS: script syntax inside a function wrapper, so top-level
    /// bindings are private and top-level `return` is legal.
    CommonJs,
}

impl ParseGoal {
    /// The source type to hand the parser. `Auto` lets oxc settle module or
    /// script; the program's `source_type` then holds the resolved kind.
    #[must_use]
    pub const fn source_type(self) -> SourceType {
        match self {
            Self::Auto => SourceType::unambiguous(),
            Self::Module => SourceType::mjs(),
            Self::Script => SourceType::script(),
            Self::CommonJs => SourceType::cjs(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraceStyle {
    Collapse,
    Expand,
    EndExpand,
    None,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent_size: 4,
            indent_char: " ".to_string(),
            indent_with_tabs: false,
            eol: "\n".to_string(),
            preserve_newlines: true,
            max_preserve_newlines: 2,
            space_after_anon_function: false,
            brace_style: BraceStyle::Collapse,
            break_webpack_imports: true,
            add_webpack_module_separators: true,
            extract_large_assets: true,
            asset_size_threshold: 10_000,
            deobfuscate: false,
            readable_names: false,
            max_line_length: 120,
            generate_source_map: false,
            source_map_file_name: None,
            split_chunks: false,
            chunk_dir: PathBuf::from("./chunks"),
            chunk_map_output: None,
            extract_modules: false,
            module_dir: PathBuf::from("./modules"),
            dependency_graph: None,
            skip_annotations: false,
            parse_goal: ParseGoal::Auto,
            isolated: false,
        }
    }
}
