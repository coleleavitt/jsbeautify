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

# Extract webpack modules to separate files
jsbeautify extract-modules bundle.js -o ./modules

# Rebuild the original source tree from a bundle sourceMappingURL
jsbeautify split-sources bundle.js -o ./out

# Generate a dependency graph (DOT format)
jsbeautify extract-modules bundle.js --graph deps.dot

# Cross-version alignment with sourcemap name recovery
jsbeautify align v1.js v2.js --sourcemap v1.js.map --align-output v2.aligned.js -o v1.aligned.js

# Recover names from Bun bundles (MR exports, this.name, displayName)
jsbeautify bundle.js --bun-extract --sourcemap bundle.js.map -o output.js
```

## CLI Reference

```
Usage: jsbeautify [OPTIONS] [FILE] [COMMAND]

Commands:
  extract-modules  Extract webpack modules into separate files
  split-chunks     Split webpack chunks into separate files
  split-sources    Reconstruct the original source tree from a bundle
  reconstruct      Rebuild original sources from a standalone .js.map file
  align            Align two bundle versions for a stable diff

Arguments:
  [FILE]  Input JavaScript file, or "-" for stdin [default: -]

Options:
  -o, --output <FILE>      Write output to a file instead of stdout
  -d, --deobfuscate        Enable AST-based deobfuscation (multi-phase pipeline)
      --rename-vars        Infer readable names for minified locals. Implies --deobfuscate
      --rename-llm         LLM-assisted naming of residual locals (env-configured). Implies --rename-vars
  -i, --indent <N>         Indentation size in spaces [default: 4]
      --tabs               Use tabs for indentation instead of spaces
      --sourcemap <FILE>   Apply original variable names from a sourcemap (.map) file
      --names-json <FILE>  Apply full name mappings from a names.json file
      --bun-extract        Recover names from Bun bundle patterns
  -h, --help               Print help
  -V, --version            Print version
```

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

The `--deobfuscate` flag runs a **Phase 0 pre-processor** followed by a **20-phase AST transformation pipeline**. Each phase feeds the next — order matters.

### Phase 0: Encrypted Eval Decryption (Pre-AST)

Before AST parsing, jsbeautify detects and decrypts a specific obfuscation pattern used by phishing kits (Tycoon2FA and similar):

```javascript
// Input: Encrypted eval pattern
var data = "NjFiMjZkZDA6MTcwNDcy:SGVsbG8gV29ybGQh...";  // Base64 with PRNG seed
var chars = ['\x23e64','\x23v05','\x23a0B','\x23l2C'];   // Steganographic "eval"
// ... PRNG XOR + Caesar cipher decryption logic ...
window[chars.map(c => c[1]).join('')](decrypted);       // eval(decrypted)

// Output: Decrypted code directly in source
console.log("Hello World!");
```

Encryption layers removed:
1. Base64 decode with colon-delimited PRNG seed and counter
2. XOR keystream via custom PRNG (seed-based)
3. Variable-shift Caesar cipher (shift values 1-25 per character)
4. Color-hex steganography for the `eval` call

This pattern was reverse-engineered from a live phishing campaign. The decrypted payload replaces the entire encrypted block in-place before AST processing continues.

### AST Transformation Phases (1-20)

| Phase | Pass | What it does |
|------:|------|-------------|
| 1 | Control flow unflattening | Reconstructs original control flow from switch-based state machines |
| 2 | String array rotation | Detects and applies array rotation (shift/push IIFE patterns) |
| 3 | Decoder / string array / dispatcher inlining | Resolves decoder functions (base64, RC4, XOR, offset) and inlines string values |
| 4 | Call proxy inlining | Detects single-use wrapper functions and inlines their targets |
| 5 | Operator proxy inlining | Resolves proxy functions that wrap binary operators |
| 6 | Expression simplification | Bracket-to-dot notation, `!0`->`true`, `void 0`->`undefined`, constant folding, algebraic simplification, strength reduction |
| 7 | Dead code elimination | Removes `if(false)`, `while(false)`, unreachable code after return/throw |
| 8 | Dead variable elimination | Removes variables that are never read |
| 9 | Function inlining | Inlines single-use functions with simple bodies |
| 10 | Array unpacking / dynamic property / ternary / try-catch | Resolves constant array indexing, simplifies constant ternaries, removes empty catch blocks |
| 11 | Unicode / boolean / void / object sparsing | Normalizes unicode identifiers, boolean literals, void expressions, sparse object patterns |
| 12 | Variable renaming | Renames hex-encoded identifiers (`_0x1a2b`) to readable names |
| 13 | Empty statement cleanup | Removes leftover empty statements from prior passes |
| 14 | Sequence expression splitting | Splits comma expressions (`a(), b(), c()`) into individual statements |
| 15 | Multi-variable splitting | Splits `let a=1, b=2, c=3` into individual declarations |
| 16 | Ternary to if/else | Converts standalone ternary expression statements to if/else blocks |
| 17 | Short-circuit to if | Converts standalone `a && b()` / `a \|\| b()` to if statements |
| 18 | IIFE unwrapping | Unwraps zero-argument arrow IIFEs into inline statements |
| 19 | esbuild helper detection | Identifies `__commonJS`, `__esm`, `__export`, `__toESM`, `__toCommonJS` runtime helpers |
| 20 | Module annotation | Labels webpack, esbuild, and Bun module boundaries with comment separators |

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

## Testing

```bash
cargo test --lib
```

## License

[MIT](LICENSE.md)
