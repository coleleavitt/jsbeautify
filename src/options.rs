use std::default::Default;
use std::path::PathBuf;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

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
    pub parse_goal: ParseGoal,
    /// Promise that nothing outside the input reads its top-level bindings,
    /// so a classic script may have them renamed or removed like module
    /// locals. Module and `CommonJS` input are already isolated.
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
    /// `CommonJS`: script syntax inside a function wrapper, so top-level
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

    /// The source type `code` is processed as. `Auto` settles on module or
    /// script by the presence of ESM syntax, or `CommonJS` when a script fails
    /// to parse only because of a top-level `return`. With `isolated`, a script
    /// becomes `CommonJS`, whose top-level bindings are private.
    #[must_use]
    pub fn resolve(self, code: &str, isolated: bool) -> SourceType {
        crate::stack::run_on_sized_stack(code.len(), || self.resolve_on_current_stack(code, isolated))
    }

    fn resolve_on_current_stack(self, code: &str, isolated: bool) -> SourceType {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, self.source_type()).parse();
        let mut source_type = parsed.program.source_type;
        if self == Self::Auto
            && source_type.is_script()
            && !parsed.diagnostics.is_empty()
            && Parser::new(&allocator, code, SourceType::cjs())
                .parse()
                .diagnostics
                .is_empty()
        {
            source_type = SourceType::cjs();
        }
        if isolated && source_type.is_script() {
            source_type = source_type.with_commonjs(true);
        }
        source_type
    }
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
            parse_goal: ParseGoal::Auto,
            isolated: false,
        }
    }
}
