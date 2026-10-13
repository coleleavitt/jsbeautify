//! js-beautify-compatible formatting flags, `.jsbeautifyrc` / `--config`
//! JSON files and `--editorconfig`.
//!
//! Precedence follows js-beautify: command-line flags, then `--config`, then
//! the nearest `.jsbeautifyrc` from the working directory upwards, then
//! `~/.jsbeautifyrc`, then defaults; `--editorconfig` overrides them all for
//! the properties it sets. Config keys may use `_` or `-`, settings under a
//! `"js"` object override top-level ones, and keys this tool does not honor
//! are ignored so one file can be shared with js-beautify.

use jsbeautify::{BraceStyle, Eol, Options};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
pub(super) struct FormatArgs {
    /// Indentation size [default: 4].
    #[arg(short = 's', long, value_name = "N", short_alias = 'i', aliases = ["indent", "indent_size"])]
    indent_size: Option<usize>,

    /// Indentation character: a space or a tab ("\t") [default: " "].
    #[arg(short = 'c', long, value_name = "CHAR", alias = "indent_char")]
    indent_char: Option<String>,

    /// Indent with one tab per level; overrides --indent-size and --indent-char.
    #[arg(short = 't', long, aliases = ["tabs", "indent_with_tabs"])]
    indent_with_tabs: bool,

    /// Initial indentation level prefixed to every line [default: 0, which
    /// keeps the input's own leading whitespace].
    #[arg(short = 'l', long, value_name = "N", alias = "indent_level")]
    indent_level: Option<usize>,

    /// Line terminator: "\n", "\r\n", "\r" or "auto" (the input's first line break) [default: auto].
    #[arg(short = 'e', long, value_name = "EOL")]
    eol: Option<String>,

    /// End the output with a newline.
    #[arg(short = 'n', long, alias = "end_with_newline")]
    end_with_newline: bool,

    /// Preserve blank lines from the input (the default; beautify without --deobfuscate only).
    #[arg(
        short = 'p',
        long,
        alias = "preserve_newlines",
        conflicts_with = "no_preserve_newlines"
    )]
    preserve_newlines: bool,

    /// Drop the input's blank lines.
    #[arg(long, alias = "no_preserve_newlines")]
    no_preserve_newlines: bool,

    /// Maximum consecutive line breaks to preserve; 0 means no limit [default: 10].
    #[arg(short = 'm', long, value_name = "N", alias = "max_preserve_newlines")]
    max_preserve_newlines: Option<usize>,

    /// Space before an anonymous function's parens: `function ()`.
    #[arg(short = 'a', long, alias = "space_after_anon_function")]
    space_after_anon_function: bool,

    /// Space before a named function's or method's parens: `function f ()`.
    #[arg(long, alias = "space_after_named_function")]
    space_after_named_function: bool,

    /// jslint-stricter mode: `function ()` and `case` aligned with `switch`.
    #[arg(short = 'j', long, alias = "jslint_happy")]
    jslint_happy: bool,

    /// Brace placement [default: collapse].
    #[arg(short = 'b', long, value_enum, value_name = "STYLE", alias = "brace_style")]
    brace_style: Option<BraceStyleArg>,

    /// Read settings from this JSON file (js-beautify .jsbeautifyrc format).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Apply the .editorconfig indent style, indent size, end of line and
    /// final newline settings of each input file.
    #[arg(long)]
    editorconfig: bool,
}

/// Opt-in trade-offs of `--deobfuscate`. Each flag can also be set by its
/// `.jsbeautifyrc` key (`keep_fn_names`, `short_export_names`,
/// `window_alias_es5`).
#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
pub(super) struct DeobfuscationArgs {
    /// Never rename a binding a function or class takes its `.name` from, so
    /// every `.name` stays as in the input (like terser's `keep_fnames`).
    #[arg(long, alias = "keep_fn_names")]
    keep_fn_names: bool,

    /// Let one or two character export names (`export { r as ab }`) name
    /// their minified locals. Off by default because code-split chunks share
    /// such names; use it for a single file whose export names are its own.
    #[arg(long, alias = "short_export_names")]
    short_export_names: bool,

    /// Window-alias propagation (`var w = window; w.document` → `document`)
    /// rewrites only names ES5-era browsers define, not ES2015 globals such
    /// as Promise, Symbol, Map or crypto.
    #[arg(long = "window-alias-es5", alias = "window_alias_es5")]
    window_alias_es5: bool,
}

impl DeobfuscationArgs {
    pub(super) fn apply(&self, chain: &ConfigChain, options: &mut Options) -> Fallible<()> {
        options.keep_fn_names = self.keep_fn_names || chain.flag("keep_fn_names")?.unwrap_or(false);
        options.short_export_names = self.short_export_names || chain.flag("short_export_names")?.unwrap_or(false);
        options.window_alias_es5 = self.window_alias_es5 || chain.flag("window_alias_es5")?.unwrap_or(false);
        Ok(())
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum BraceStyleArg {
    Collapse,
    Expand,
    EndExpand,
}

impl From<BraceStyleArg> for BraceStyle {
    fn from(style: BraceStyleArg) -> Self {
        match style {
            BraceStyleArg::Collapse => Self::Collapse,
            BraceStyleArg::Expand => Self::Expand,
            BraceStyleArg::EndExpand => Self::EndExpand,
        }
    }
}

/// Settings loaded from config files, highest priority first.
pub(super) struct ConfigChain {
    files: Vec<(PathBuf, Map<String, Value>)>,
    brace_style: Option<BraceStyle>,
}

impl FormatArgs {
    pub(super) fn load_config(&self) -> Fallible<ConfigChain> {
        let home = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".jsbeautifyrc"));
        let nearest = std::env::current_dir()?
            .ancestors()
            .map(|dir| dir.join(".jsbeautifyrc"))
            .find(|path| path.is_file());
        let mut chain = Vec::new();
        for path in [self.config.clone(), nearest, home.filter(|path| path.is_file())]
            .into_iter()
            .flatten()
        {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
            let Value::Object(map) = value else {
                return Err(format!("{}: expected a JSON object", path.display()).into());
            };
            chain.push((path, normalize(map)));
        }
        let mut chain = ConfigChain {
            files: chain,
            brace_style: None,
        };
        chain.brace_style = chain
            .string("brace_style")?
            .as_deref()
            .map(parse_config_brace_style)
            .transpose()?;
        Ok(chain)
    }

    /// Options for one input. `target` is the file whose `.editorconfig`
    /// applies (the input, or the output path / `stdin.js` for stdin).
    pub(super) fn options(&self, chain: &ConfigChain, base: Options, target: &Path) -> Fallible<Options> {
        let mut options = base;
        if let Some(n) = self
            .indent_size
            .map_or_else(|| chain.number("indent_size"), |n| Ok(Some(n)))?
        {
            options.indent_size = n;
        }
        if let Some(c) = self
            .indent_char
            .clone()
            .map_or_else(|| chain.string("indent_char"), |c| Ok(Some(c)))?
        {
            options.indent_with_tabs = parse_indent_char(&c)?;
        }
        if self.indent_with_tabs || chain.flag("indent_with_tabs")?.unwrap_or(false) {
            options.indent_with_tabs = true;
        }
        if let Some(n) = self
            .indent_level
            .map_or_else(|| chain.number("indent_level"), |n| Ok(Some(n)))?
        {
            options.indent_level = n;
        }
        if let Some(eol) = self.eol.clone().map_or_else(|| chain.string("eol"), |e| Ok(Some(e)))? {
            options.eol = parse_eol(&eol)?;
        }
        options.end_with_newline = self.end_with_newline || chain.flag("end_with_newline")?.unwrap_or(false);
        options.preserve_newlines = if self.no_preserve_newlines {
            false
        } else {
            self.preserve_newlines || chain.flag("preserve_newlines")?.unwrap_or(true)
        };
        if let Some(n) = self
            .max_preserve_newlines
            .map_or_else(|| chain.number("max_preserve_newlines"), |n| Ok(Some(n)))?
        {
            options.max_preserve_newlines = n;
        }
        options.space_after_anon_function =
            self.space_after_anon_function || chain.flag("space_after_anon_function")?.unwrap_or(false);
        options.space_after_named_function =
            self.space_after_named_function || chain.flag("space_after_named_function")?.unwrap_or(false);
        options.jslint_happy = self.jslint_happy || chain.flag("jslint_happy")?.unwrap_or(false);
        if let Some(style) = self.brace_style {
            options.brace_style = style.into();
        } else if let Some(style) = chain.brace_style {
            options.brace_style = style;
        }
        if self.editorconfig {
            apply_editorconfig(&mut options, target)?;
        }
        Ok(options)
    }
}

impl ConfigChain {
    fn get(&self, key: &str) -> Option<(&Path, &Value)> {
        self.files
            .iter()
            .find_map(|(path, map)| map.get(key).map(|value| (path.as_path(), value)))
    }

    fn number(&self, key: &str) -> Fallible<Option<usize>> {
        let Some((path, value)) = self.get(key) else {
            return Ok(None);
        };
        let number = match value {
            Value::Number(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        };
        number
            .map(Some)
            .ok_or_else(|| format!("{}: {key} must be a non-negative integer", path.display()).into())
    }

    fn flag(&self, key: &str) -> Fallible<Option<bool>> {
        match self.get(key) {
            None => Ok(None),
            Some((_, Value::Bool(b))) => Ok(Some(*b)),
            Some((path, _)) => Err(format!("{}: {key} must be true or false", path.display()).into()),
        }
    }

    fn string(&self, key: &str) -> Fallible<Option<String>> {
        match self.get(key) {
            None => Ok(None),
            Some((_, Value::String(s))) => Ok(Some(s.clone())),
            Some((path, _)) => Err(format!("{}: {key} must be a string", path.display()).into()),
        }
    }
}

/// `-` in keys becomes `_`, and the `"js"` section overrides top-level keys.
fn normalize(map: Map<String, Value>) -> Map<String, Value> {
    let mut merged: Map<String, Value> = map.into_iter().map(|(k, v)| (k.replace('-', "_"), v)).collect();
    if let Some(Value::Object(js)) = merged.remove("js") {
        merged.extend(js.into_iter().map(|(k, v)| (k.replace('-', "_"), v)));
    }
    merged
}

/// js-beautify accepts the escapes `\t`, `\n`, `\r` written literally.
fn unescape(value: &str) -> String {
    value.replace("\\r", "\r").replace("\\n", "\n").replace("\\t", "\t")
}

fn parse_indent_char(value: &str) -> Fallible<bool> {
    match unescape(value).as_str() {
        " " => Ok(false),
        "\t" => Ok(true),
        other => Err(format!("indent char must be a space or a tab, got {other:?}").into()),
    }
}

fn parse_eol(value: &str) -> Fallible<Eol> {
    match unescape(value).as_str() {
        "auto" => Ok(Eol::Auto),
        "\n" => Ok(Eol::Lf),
        "\r\n" => Ok(Eol::CrLf),
        "\r" => Ok(Eol::Cr),
        other => Err(format!("eol must be \\n, \\r\\n, \\r or auto, got {other:?}").into()),
    }
}

/// js-beautify brace styles are comma lists where the last style wins
/// (`"collapse,preserve-inline"`); `preserve-inline` and `none` are not
/// supported and are skipped with a warning.
fn parse_config_brace_style(value: &str) -> Fallible<BraceStyle> {
    let mut style = BraceStyle::Collapse;
    for part in value.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
        style = match part {
            "" => continue,
            "collapse" => BraceStyle::Collapse,
            "expand" => BraceStyle::Expand,
            "end-expand" => BraceStyle::EndExpand,
            "preserve-inline" | "none" => {
                eprintln!("[jsbeautify] brace_style {part:?} is not supported; ignoring it");
                continue;
            }
            other => return Err(format!("unknown brace_style {other:?}").into()),
        };
    }
    Ok(style)
}

fn apply_editorconfig(options: &mut Options, target: &Path) -> Fallible<()> {
    use ec4rs::property::{EndOfLine, FinalNewline, IndentSize, IndentStyle};

    let target = std::path::absolute(target)?;
    let properties = ec4rs::properties_of(&target)?;
    match properties.get::<IndentStyle>() {
        Ok(IndentStyle::Tabs) => options.indent_with_tabs = true,
        Ok(IndentStyle::Spaces) => options.indent_with_tabs = false,
        Err(_) => {}
    }
    if let Ok(IndentSize::Value(n)) = properties.get::<IndentSize>() {
        options.indent_size = n;
    }
    if let Ok(FinalNewline::Value(b)) = properties.get::<FinalNewline>() {
        options.end_with_newline = b;
    }
    match properties.get::<EndOfLine>() {
        Ok(EndOfLine::Lf) => options.eol = Eol::Lf,
        Ok(EndOfLine::CrLf) => options.eol = Eol::CrLf,
        Ok(EndOfLine::Cr) => options.eol = Eol::Cr,
        Err(_) => {}
    }
    Ok(())
}
