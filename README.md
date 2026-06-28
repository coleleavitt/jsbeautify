# js-beautify-rs

[![Crates.io](https://img.shields.io/crates/v/js-beautify-rs.svg)](https://crates.io/crates/js-beautify-rs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)

A fast JavaScript beautifier and deobfuscator written in Rust, powered by [oxc](https://github.com/oxc-project/oxc).

Takes minified, obfuscated webpack/esbuild/Bun bundles and produces readable JavaScript.
Handles real-world production bundles — tested against 11MB+ builds.

## Repository layout

This is a monorepo with a Rust **backend** and a browser-extension **frontend**:

```
js-beautify-rs/
├── src/            # Rust crate: beautifier, deobfuscator, analyzers, daemon
├── Cargo.toml      # `jsbeautify` CLI + `jsbeautify --serve` ingestion daemon
└── frontend/       # Mindemon — Firefox DevTools extension (browser capture)
    ├── manifest.json, panel/, lib/, test/
    └── README.md   # extension-specific docs
```

The backend ingests assets the frontend captures (HTTP `:3333`) and streams
results back over WebSocket (`:9222`). See `INTEGRATION.md` for the full data
flow and `frontend/README.md` for the extension.

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

# Beautify + deobfuscate (20-phase AST pipeline)
jsbeautify input.js --deobfuscate -o output.js

# Pipe from stdin
cat bundle.js | jsbeautify - > output.js

# Extract webpack modules to separate files
jsbeautify bundle.js --extract-modules --module-dir ./modules

# Rebuild the original source tree from a bundle sourceMappingURL
jsbeautify bundle.js --split-sources ./out

# Generate dependency graph (DOT format)
jsbeautify bundle.js --extract-modules --dependency-graph deps.dot

# Cross-version alignment with sourcemap name recovery
jsbeautify v1.js --sourcemap v1.js.map --align-with v2.js --align-output v2.aligned.js -o v1.aligned.js

# Extract names from Bun bundles (MR exports, this.name, displayName)
jsbeautify bundle.js --bun-extract --sourcemap bundle.js.map -o output.js
```

## CLI Reference

```
Usage: jsbeautify [OPTIONS] <FILE>

Arguments:
  <FILE>  Input JavaScript file (use "-" for stdin)

Options:
  -o, --output <FILE>            Write output to a file instead of stdout
  -d, --deobfuscate              Enable AST-based deobfuscation (20-phase pipeline)
      --split-chunks             Split webpack chunks into separate files
      --chunk-dir <DIR>          Directory for chunk output [default: ./chunks]
      --chunk-map <FILE>         Write chunk metadata to a JSON file
      --extract-modules          Extract webpack modules to separate files
      --module-dir <DIR>         Directory for module output [default: ./modules]
      --dependency-graph <FILE>  Generate a dependency graph in DOT format
      --split-sources <DIR>      Reconstruct source tree from bundle sourceMappingURL
      --source-maps              Generate source maps
      --sourcemap <FILE>         Sourcemap for extracting original variable names
      --names-json <FILE>        Name mappings from extract-names.ts
      --align-with <FILE>        Second bundle to align with (produces stable diffs)
      --align-output <FILE>      Output path for the aligned second bundle
      --raw                      Skip beautification, output raw aligned code
      --bun-extract              Extract names from Bun bundle patterns
      --indent-size <N>          Indentation size in spaces [default: 4]
      --indent-with-tabs         Use tabs for indentation instead of spaces
  -h, --help                     Print help
  -V, --version                  Print version
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
jsbeautify v2.1.88.js \
  --sourcemap v2.1.88.js.map \
  --align-with v2.1.96.js \
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

## Serve daemon (browser ingestion backend)

`jsbeautify` is not only a CLI — `--serve` boots a long-running daemon that wires
together the proxy, WebSocket server, project DB, hooks, and monitoring into one
process and exposes an HTTP ingestion API. This is the analysis backend for the
[Mindemon](https://github.com/coleleavitt/mindemon) Firefox DevTools extension,
which captures every JS/CSS/source-map asset a page loads and POSTs them here.

```bash
# Boot the daemon (HTTP :3333, WebSocket :9222, MITM proxy :8080)
jsbeautify --serve --project-name acme

# With TLS interception + a monitoring loop every 5 min
jsbeautify --serve --project-name acme --intercept-tls --monitor-interval 300
```

Every ingested asset flows through one shared pipeline:

```
POST /ingest ─▶ classify ─▶ beautify (+deobfuscate if minified) ─▶ analyzers
            ─▶ reverse source maps (sourcesContent → originals, re-analyzed)
            ─▶ persist file + matches to SQLite ─▶ broadcast over WebSocket
```

Files + DB live under `~/.jsbeautify/projects/<name>/` (`raw/`, `beautified/`,
`reversed/`, `project.db`).

### Ports

| Port  | Service       | Purpose |
|-------|---------------|---------|
| 3333  | HTTP API      | Ingestion (`/ingest`, `/ingest/batch`), `/matches`, `/health`, `/ca.crt`. |
| 9222  | WebSocket     | Live `js_files_update` / `matches_update` events to editor clients. |
| 8080  | MITM proxy    | Passive capture; in-scope JS/CSS/HTML feeds the same pipeline. |

### HTTP ingestion contract

`POST /ingest` — one asset:

```json
{
  "url": "https://site/app.js",
  "content_type": "application/javascript",
  "content": "…source or base64…",
  "encoding": "utf8",                 // or "base64" for binary
  "source_map_content": "{…}",        // optional inline source map JSON
  "source": "fs_ingest"               // fs_ingest | proxy | cli
}
```

`POST /ingest/batch` — a full browser dump: `{ "target": "https://site/", "files": [ <asset>, … ] }`.

Response (per asset): `{ url, kind, file_id, stored, findings, reversed_sources, relationships, error }`.

`GET /matches` returns recent stored findings as JSON. `GET /ca.crt` serves the
MITM CA PEM (when `--intercept-tls` is on) so a browser can trust the proxy.

All responses carry permissive CORS headers so the `moz-extension://` panel can
call the API cross-origin.

## Testing

```bash
cargo test --lib
```

## License

[MIT](LICENSE.md)
