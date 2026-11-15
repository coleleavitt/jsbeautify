use std::default::Default;
use std::path::PathBuf;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Class, Expression, Function, IdentifierReference, Program, StaticMemberExpression, ThisExpression, UnaryExpression,
    UnaryOperator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::Scoping;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeFlags;
use rustc_hash::FxHashSet;

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
    /// Never rename a binding a function or class takes its `.name` from
    /// (like terser's `keep_fnames` / `keep_classnames`).
    pub keep_fn_names: bool,
    /// Let a one or two character export name (`export { r as ab }`) become
    /// its minified local's name. Off by default because code-split chunks
    /// share such names between files, where they say nothing about the
    /// value; on for a single file whose short export names are its own.
    pub short_export_names: bool,
    /// Window-alias propagation (`var w = window; w.document` → `document`)
    /// rewrites only names ES5-era browsers (Internet Explorer 9) define, not
    /// ES2015 globals such as `Promise`, `Symbol`, `Map` or `crypto`.
    pub window_alias_es5: bool,
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
    /// becomes `CommonJS`, whose top-level bindings are private, unless it
    /// reads one of them through the global object itself.
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
        if isolated && source_type.is_script() && !reads_own_globals(&parsed.program) {
            source_type = source_type.with_commonjs(true);
        }
        source_type
    }
}

/// Whether a script may read one of its own top-level bindings through the
/// global object. `--isolated` only promises that other scripts do not.
///
/// A script without top-level bindings has nothing to read. Otherwise the
/// rule is deliberately simple: the script counts as reading them when it
/// uses an unresolved global-object name (`globalThis`, `window`, `self`,
/// `global`, `frames`, `top`, `parent`) or a top-level `this` in any way
/// other than `.name` with a name that is not one of its top-level bindings
/// or `typeof`,
/// or when it references an unresolved `eval` or `Function` (indirect eval,
/// `new Function`, and `Function("return this")()` reach the global object).
fn reads_own_globals(program: &Program<'_>) -> bool {
    let scoping = crate::ast_deobfuscate::semantic::scoping(program);
    let globals: FxHashSet<&str> = scoping
        .get_bindings(scoping.root_scope_id())
        .keys()
        .map(oxc_str::Ident::as_str)
        .collect();
    // Without top-level bindings, no read through the global object can name
    // one (a webpack bundle's single IIFE).
    if globals.is_empty() {
        return false;
    }
    let mut reads = GlobalReads {
        scoping: &scoping,
        globals: &globals,
        function_depth: 0,
        found: false,
    };
    reads.visit_program(program);
    reads.found
}

struct GlobalReads<'s> {
    scoping: &'s Scoping,
    globals: &'s FxHashSet<&'s str>,
    /// Non-arrow functions and classes around the visit, where `this` is not
    /// the global object.
    function_depth: usize,
    found: bool,
}

impl GlobalReads<'_> {
    fn is_unresolved(&self, id: &IdentifierReference<'_>) -> bool {
        id.reference_id
            .get()
            .is_some_and(|reference| self.scoping.get_reference(reference).symbol_id().is_none())
    }

    fn is_global_object(&self, expr: &Expression<'_>) -> bool {
        match expr {
            Expression::ThisExpression(_) => self.function_depth == 0,
            Expression::Identifier(id) => {
                matches!(
                    id.name.as_str(),
                    "globalThis" | "window" | "self" | "global" | "frames" | "top" | "parent"
                ) && self.is_unresolved(id)
            }
            Expression::ParenthesizedExpression(paren) => self.is_global_object(&paren.expression),
            _ => false,
        }
    }
}

impl<'a> Visit<'a> for GlobalReads<'_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        self.function_depth += 1;
        walk::walk_function(self, func, flags);
        self.function_depth -= 1;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.function_depth += 1;
        walk::walk_class(self, class);
        self.function_depth -= 1;
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        // `window.addEventListener` cannot read one of the script's bindings.
        if self.is_global_object(&member.object) && !self.globals.contains(member.property.name.as_str()) {
            return;
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_unary_expression(&mut self, unary: &UnaryExpression<'a>) {
        // `typeof window` tells nothing about the script's bindings.
        if unary.operator == UnaryOperator::Typeof && self.is_global_object(&unary.argument) {
            return;
        }
        walk::walk_unary_expression(self, unary);
    }

    fn visit_this_expression(&mut self, _this: &ThisExpression) {
        self.found |= self.function_depth == 0;
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if matches!(
            id.name.as_str(),
            "globalThis" | "window" | "self" | "global" | "frames" | "top" | "parent" | "eval" | "Function"
        ) && self.is_unresolved(id)
        {
            self.found = true;
        }
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
            keep_fn_names: false,
            short_export_names: false,
            window_alias_es5: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ParseGoal;

    fn isolated_kind(code: &str) -> &'static str {
        let source_type = ParseGoal::Auto.resolve(code, true);
        if source_type.is_commonjs() {
            "commonjs"
        } else {
            "script"
        }
    }

    #[test]
    fn isolated_scripts_become_commonjs() {
        for code in [
            "var a = 1; console.log(a, typeof globalThis.setTimeout);",
            "var a = 1; window.b = 2;",
            "var a = 1; function f() { return this.a; }",
            "var a = 1; class C { m() { return this.a; } }",
            "var a = 1; typeof window === 'undefined';",
            // No top-level binding for a global-object read to name.
            "(function () { var a = 1; return [globalThis, this, Function('return this')()]; })();",
        ] {
            assert_eq!(isolated_kind(code), "commonjs", "{code}");
        }
    }

    #[test]
    fn isolated_scripts_reading_their_own_globals_stay_scripts() {
        for code in [
            "var x = 1; globalThis.x;",
            "var x = 1; window['x'] = 2;",
            "var x = 1; this.x;",
            "var x = 1; (() => this.x)();",
            "var x = 1; self[key];",
            "var x = 1; globalThis[0];",
            "var x = 1; var { x: y } = globalThis;",
            "var x = 1; 'x' in globalThis;",
            "var x = 1; Object.getOwnPropertyDescriptor(globalThis, 'x');",
            "var x = 1; (0, eval)('x');",
            "var x = 1; new Function('return x')();",
            "var x = 1; Function('return this')().x;",
            "var x = 1; var w = window; w.x;",
            "var x = 1; (function () { return globalThis; })();",
        ] {
            assert_eq!(isolated_kind(code), "script", "{code}");
        }
    }
}
