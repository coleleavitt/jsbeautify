# js-beautify-rs

[![Crates.io](https://img.shields.io/crates/v/js-beautify-rs.svg)](https://crates.io/crates/js-beautify-rs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)

A fast JavaScript beautifier and deobfuscator written in Rust, powered by [oxc](https://github.com/oxc-project/oxc).

Takes minified, obfuscated webpack/esbuild/Bun bundles and produces readable JavaScript.
Handles real-world production bundles — tested against 11MB+ builds.

## What it does

Takes a minified/obfuscated webpack, esbuild, or Bun bundle and produces
readable JavaScript. AST deobfuscation, webpack module/chunk extraction,
source-tree reconstruction from source maps, and cross-version alignment are
exposed as subcommands (see the CLI reference below).

## Installation

```bash
cargo install js-beautify-rs
```

Or build from source:

```bash
git clone https://github.com/coleleavitt/js-beautify-rs
cd js-beautify-rs
cargo build --release
# binary at ./target/release/jsbeautify
```

## Quick Start

```bash
# Beautify minified JavaScript
jsbeautify input.js -o output.js

# Beautify + deobfuscate (multi-phase AST pipeline)
jsbeautify input.js -d -o output.js

# Pipe from stdin
cat bundle.js | jsbeautify - > output.js

# Extract webpack or Turbopack (Next.js) modules to separate files
jsbeautify extract-modules bundle.js -o ./modules

# Rebuild the original source tree from a bundle sourceMappingURL
jsbeautify split-sources bundle.js -o ./out

# Generate a dependency graph (DOT format)
jsbeautify extract-modules bundle.js --graph deps.dot

# Cross-version alignment with sourcemap name recovery
jsbeautify align v1.js v2.js --sourcemap v1.js.map --align-output v2.aligned.js -o v1.aligned.js

# Recover names from Bun bundles (MR exports, this.name, displayName)
jsbeautify bundle.js --bun-extract --sourcemap bundle.js.map -o output.js

# Bounded, static inspection of a JavaScript-hosted VM and literal embedded WebAssembly.
# JavaScript is never executed; WebAssembly is validated but never instantiated.
jsbeautify analyze-vm bundle.js -o vm-report.json --extract-wasm ./vm-wasm

# Inspect what oxc sees, with the exact oxc revision the passes use
jsbeautify ast input.js > ast.json                 # ESTree JSON (--ts for TypeScript/JSX)
jsbeautify scope input.js | jq '.unresolved'       # oxc_semantic symbol table
jsbeautify cfg input.js --function main | dot -Tsvg > main.svg
```

## CLI Reference

```
Usage: jsbeautify [OPTIONS] [FILE]... [COMMAND]

Commands:
  extract-modules  Extract webpack or Turbopack modules into separate files
  split-chunks     Split webpack chunks, or Turbopack chunks' modules, into separate files
  split-sources    Reconstruct the original source tree from a bundle
  reconstruct      Rebuild original sources from a standalone .js.map file
  align            Align two bundle versions for a stable diff
  analyze-vm       Statically inspect a JavaScript-hosted VM and embedded WebAssembly
  ast              Print the program as ESTree JSON (oxc's serializer)
  scope            Print the oxc_semantic symbol table as JSON
  cfg              Print the oxc_semantic control-flow graph

Arguments:
  [FILE]...  Input JavaScript files, or "-" for stdin [default: -]. Several files
             are beautified in place, as with --replace

Options:
  -f, --file <FILE>                 Input file; may be repeated (same as a positional FILE)
  -o, --output <FILE>               Write output to a file instead of stdout
  -r, --replace                     Write output in place, replacing each input file
  -q, --quiet                       Do not report written files on stderr
  -d, --deobfuscate                 Enable AST-based deobfuscation (multi-phase pipeline)
      --rename-vars                 Infer readable names for minified locals. Implies --deobfuscate
      --rename-llm                  LLM-assisted naming of residual locals (env-configured). Implies --rename-vars
  -s, --indent-size <N>             Indentation size [default: 4] (also -i, --indent)
  -c, --indent-char <CHAR>          Indentation character: a space or a tab ("\t")
  -t, --indent-with-tabs            One tab per indentation level (also --tabs)
  -l, --indent-level <N>            Initial indentation level [default: 0 = keep the input's leading whitespace]
  -e, --eol <EOL>                   "\n", "\r\n", "\r" or "auto" (the input's first line break) [default: auto]
  -n, --end-with-newline            End the output with a newline
  -p, --preserve-newlines           Preserve blank lines from the input (default; not with --deobfuscate)
      --no-preserve-newlines        Drop the input's blank lines
  -m, --max-preserve-newlines <N>   Maximum consecutive line breaks to preserve; 0 = no limit [default: 10]
  -a, --space-after-anon-function   function () instead of function()
      --space-after-named-function  function f () and m () {} for methods
  -j, --jslint-happy                function () plus case labels aligned with their switch
  -b, --brace-style <STYLE>         collapse | expand | end-expand [default: collapse]
      --config <FILE>               Read settings from a JSON file (.jsbeautifyrc format)
      --editorconfig                Apply .editorconfig indent, end-of-line and final-newline settings
      --sourcemap <FILE>            Apply original variable names from a sourcemap (.map) file
      --names-json <FILE>           Apply full name mappings from a names.json file
      --bun-extract                 Recover names from Bun bundle patterns
      --source-type <TYPE>          auto | module | script | commonjs [default: auto]
      --isolated                    Allow renaming/removing a classic script's top-level bindings
  -h, --help                        Print help
  -V, --version                     Print version (also -v)
```

### js-beautify compatibility

The formatting flags use js-beautify's names, short flags and defaults, and
long flags also accept js-beautify's underscore spelling (`--indent_size`).
Output has no trailing newline unless `-n` is given, as in js-beautify.

Settings are read, highest priority first, from command-line flags,
`--config FILE`, the nearest `.jsbeautifyrc` from the working directory
upwards, and `~/.jsbeautifyrc`. Config keys may use `_` or `-`; a `"js"`
section overrides top-level keys. Keys this tool does not honor (CSS/HTML
options, `wrap_line_length`, ...) are ignored, so a file can be shared with
js-beautify. `--editorconfig` overrides all of these for the properties the
`.editorconfig` sets.

Differences from js-beautify:

- Short flags that collide keep this tool's meaning: `-d` is
  `--deobfuscate` (js-beautify: HTML `--inline`) and `-i` is the indent size
  (js-beautify: HTML `--wrap-attributes-indent-size`). `-o` means the same in
  both.
- Output is printed by oxc's code generator: it normalizes quotes to `"`,
  number literals, redundant parentheses and string escapes, and always
  terminates statements with `;`. js-beautify keeps the input's tokens.
- Only JavaScript is supported (no `--type css|html`).
- Not supported: `--brace-style none` and `preserve-inline`,
  `--wrap-line-length`, `--break-chained-methods`,
  `--unindent-chained-methods`, `--keep-array-indentation`,
  `--space-in-paren`, `--space-in-empty-paren`, `--comma-first`,
  `--operator-position`, `--indent-empty-lines`, `--unescape-strings`
  (escapes are always decoded), `--e4x`, `--templating`.

### Inspection commands

`ast`, `scope` and `cfg` take `FILE` (or `-` for stdin), `-o FILE`, and `--ts`
(parse as TypeScript with JSX; the default is module JavaScript, as in the
deobfuscation pipeline). Recoverable parse and semantic errors are printed to
stderr; an unrecoverable parse error prints the diagnostics and exits 1.

- `ast` prints oxc's ESTree JSON. `start`/`end` are UTF-8 byte offsets.
- `scope` prints `scopes` (id, parent, owning AST node kind, flags),
  `symbols` (name, kind such as `var`/`let`/`const`/`param`/`function`/
  `class`/`catch`/`import`, raw oxc flags, scope, declaration position, and
  every resolved reference with its `read`/`write`/`read_write` access), and
  `unresolved` (global names with counts and references). Lines and columns
  are 1-based; columns count Unicode characters.
- `cfg` prints the control-flow graph as Graphviz DOT, or JSON with
  `--format json`. `--function NAME` keeps only the functions with that name
  (declared, or bound to a variable, property, method or assignment target),
  without the functions nested inside them.

## Library Usage

js-beautify-rs can also be used as a Rust library:

```rust
use js_beautify_rs::{beautify, Options};

let code = "function test(){console.log('hello');}";
let options = Options::default();
let beautified = beautify(code, &options).expect("beautification failed");
```

For deobfuscation:

```rust
use js_beautify_rs::AstDeobfuscator;

let obfuscated = std::fs::read_to_string("bundle.js").unwrap();
let mut deobfuscator = AstDeobfuscator::new();
let clean = deobfuscator.deobfuscate(&obfuscated).unwrap();
```

## Deobfuscation Pipeline

`--deobfuscate` is an ordered AST pipeline. The implementation is split by
lifetime, not into one source file per numbered phase:

1. **Pre-AST input handling** decrypts supported encrypted `eval` payloads.
2. **Structural arena (Phases 0–12.5)** parses with Oxc, runs Akamai and Bun
   prepasses, control-flow and string decoding, proxy/function simplification,
   dead-code/dead-variable sweeps, normalization, and variable naming.
3. **Phase 12.6 checkpoint** code-generates the structural result, drops the
   Oxc bump arena, and re-parses into a fresh arena. This bounds peak memory and
   rejects invalid generated JavaScript.
4. **Cosmetic-A arena (Phases 13–15)** removes empty statements, splits sequence
   and multi-variable statements, and runs another validated dead-variable
   sweep.
5. **Phase 15.6 checkpoint** repeats the codegen, arena reset, and strict
   re-parse before the final transforms.
6. **Cosmetic-B arena (Phases 16–19.5)** converts statement-level ternaries,
   `switch(true)`, and short circuits; evaluates safe literal `JSON.parse`
   calls; normalizes optional/nullish syntax; unwraps safe IIFEs; detects
   esbuild helpers; and performs the final dead-variable sweep.
7. **Final output (Phases 19.6–21.5)** validates AST codegen, adds webpack,
   esbuild, Bun, enum, namespace, and arrow annotations, then validates the
   post-annotation JavaScript again.

The checkpoints are deliberate correctness and memory boundaries. State that
must survive them (currently the Bun helper map and source type) is passed
explicitly; all arena-owned AST data is discarded. See
`src/ast_deobfuscate/engine/structural.rs`,
`src/ast_deobfuscate/engine/cosmetic.rs`, and
`src/ast_deobfuscate/engine/final_output.rs` for the authoritative order.

## Cross-Version Alignment

Produces stable diffs between versions of minified bundles by recovering original names and matching statements across versions:

```bash
# Align two bundle versions using sourcemap name recovery
jsbeautify align v2.1.88.js v2.1.96.js \
  --sourcemap v2.1.88.js.map \
  --align-output v2.1.96.aligned.js \
  -o v2.1.88.aligned.js
```

Three-tier canonical naming:
1. **Sourcemap names** — original identifiers recovered from `.map` files (36,000+ names)
2. **Slot-based names** — Bun alphabet-derived `sN` names for unmapped identifiers
3. **Hash-based names** — AST structure hash fallback `_rN` for unmatched code

Results: 94.7% statement match rate, 76% diff reduction between bundle versions.

## Bun Bundle Support

Extracts original names from Bun-specific bundle patterns:

```bash
jsbeautify bundle.js --bun-extract --sourcemap bundle.js.map -o output.js
```

Patterns detected:
- `MR(target, { exportName: () => minifiedVar })` export mappings
- `this.name = "ClassName"` in class constructors
- `Component.displayName = "ComponentName"` assignments

## Development and testing

This repository follows upstream Rust and Oxc rather than advertising a fixed
minimum Rust version. There is intentionally no `rust-version` package key and
no `rust-toolchain.toml`: use a current `rustc`/Cargo toolchain that can build
the pinned Oxc revision in `Cargo.toml`, and update that Oxc revision and the
code together.

Run the full local gate across library, binary, and the single aggregated
integration-test target:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Targeted checks:

```bash
# Static VM/Wasm analyzer integration tests (never executes target JavaScript)
cargo test --test integration analyze_vm
```

`Cargo.toml` sets `autotests = false` and declares `tests/integration/main.rs` as
the integration-test aggregator, so `--all-targets` is the least ambiguous way
to exercise every declared target.

## License

[MIT](LICENSE.md)
