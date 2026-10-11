use std::default::Default;
use std::path::PathBuf;

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Options {
    pub indent_size: usize,
    /// One tab per indentation level; `indent_size` is then ignored.
    pub indent_with_tabs: bool,
    /// Extra indentation levels prefixed to every line. When zero, the
    /// input's own leading whitespace is used as the base indent.
    pub indent_level: usize,
    pub eol: Eol,
    pub end_with_newline: bool,
    pub preserve_newlines: bool,
    /// Cap on preserved consecutive line breaks; `0` means no cap.
    pub max_preserve_newlines: usize,
    pub space_after_anon_function: bool,
    pub space_after_named_function: bool,
    /// `function ()` spacing plus `case` labels aligned with their `switch`.
    pub jslint_happy: bool,
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
    pub generate_source_map: bool,
    pub source_map_file_name: Option<String>,
    pub split_chunks: bool,
    pub chunk_dir: PathBuf,
    pub chunk_map_output: Option<PathBuf>,
    pub extract_modules: bool,
    pub module_dir: PathBuf,
    pub dependency_graph: Option<PathBuf>,
    pub skip_annotations: bool,
}

/// Line terminator written between output lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    /// The input's first line break (`\r\n`, `\r` or `\n`), else `\n`.
    Auto,
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    #[must_use]
    pub fn resolve(self, input: &str) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
            Self::Auto => match input.find(['\r', '\n']) {
                Some(i) if input[i..].starts_with("\r\n") => "\r\n",
                Some(i) if input.as_bytes()[i] == b'\r' => "\r",
                _ => "\n",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraceStyle {
    /// Opening braces stay on the statement line: `if (a) {`.
    Collapse,
    /// Opening braces of blocks, bodies and classes go on their own line;
    /// `else` / `catch` / `finally` start a new line.
    Expand,
    /// Like `Collapse`, but `else` / `catch` / `finally` start a new line.
    EndExpand,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent_size: 4,
            indent_with_tabs: false,
            indent_level: 0,
            eol: Eol::Auto,
            end_with_newline: false,
            preserve_newlines: true,
            max_preserve_newlines: 10,
            space_after_anon_function: false,
            space_after_named_function: false,
            jslint_happy: false,
            brace_style: BraceStyle::Collapse,
            break_webpack_imports: true,
            add_webpack_module_separators: true,
            extract_large_assets: true,
            asset_size_threshold: 10_000,
            deobfuscate: false,
            readable_names: false,
            generate_source_map: false,
            source_map_file_name: None,
            split_chunks: false,
            chunk_dir: PathBuf::from("./chunks"),
            chunk_map_output: None,
            extract_modules: false,
            module_dir: PathBuf::from("./modules"),
            dependency_graph: None,
            skip_annotations: false,
        }
    }
}
