# Quick Reference: jsbeautify

## 🚀 Common Commands

### Build & Test
```bash
# Build release binary
cargo build --release

# Run all tests
cargo test --lib

# Check for warnings
cargo build --release 2>&1 | grep warning

# Run specific test
cargo test test_name
```

### Using the Tool

#### Basic Beautification
```bash
./target/release/jsbeautify input.js -o output.js
```

#### With Deobfuscation (All 4 Passes)
```bash
./target/release/jsbeautify input.js --deobfuscate -o output.js
```

#### Webpack Chunk Detection
```bash
# Extract chunk metadata to JSON
./target/release/jsbeautify app.js --split-chunks --chunk-map chunks.json

# Split embedded chunks (if bundle contains code)
./target/release/jsbeautify app.js --split-chunks --chunk-dir ./chunks/
```

#### Pipeline Input
```bash
cat input.js | ./target/release/jsbeautify - > output.js
```

### Analyzing Results

#### Compare Before/After
```bash
# Line counts
wc -l original.js deobfuscated.js

# Property access patterns
grep -o '\.[a-zA-Z_][a-zA-Z0-9_]*' deobfuscated.js | wc -l

# Remaining obfuscation
grep -c 'try {' deobfuscated.js
```

#### View Chunk Metadata
```bash
python3 -m json.tool chunks.json | head -50
```

## 📊 Deobfuscation Passes

The `--deobfuscate` flag enables 15 transformation passes:

1. Inline decoded strings
2. Unflatten control flow
3. Simplify expressions
4. Fold constants
5. Remove dead code
6. Remove dead conditionals
7. Inline object dispatchers
8. Inline call proxies
9. Inline operator proxies
10. **Inline single-use functions** ⬅️ NEW
11. **Convert dynamic properties** ⬅️ NEW (obj["x"] → obj.x)
12. **Remove empty try-catch** ⬅️ NEW
13. **Simplify ternary chains** ⬅️ NEW (true ? a : b → a)
14. Consolidate sparse objects
15. Normalize unicode
16. Replace boolean literals (!0 → true)
17. Replace void(0) (→ undefined)

## 🎯 Performance Benchmarks

Based on Intelius bundles:
- Small files (<50KB): ~0.01s
- Medium files (500KB-1MB): ~0.1s
- Large files (5-6MB): ~0.09s
- Very large (3-4MB vendors): ~7.7s

## 📁 Project Structure

```
src/
├── bin/
│   └── jsbeautify.rs           - CLI interface
├── deobfuscate/
│   ├── mod.rs                  - Pipeline orchestration
│   ├── function_inline.rs      - Single-use function inlining
│   ├── dynamic_property.rs     - Property access conversion
│   ├── try_catch.rs            - Empty try-catch removal
│   ├── ternary.rs              - Constant ternary simplification
│   └── [11 other passes]
├── chunk_detector.rs           - Webpack metadata extraction
├── chunk_splitter.rs           - Chunk file writing
└── beautifier/
    ├── mod.rs                  - Main beautification logic
    └── webpack.rs              - Webpack integration
```

## 🔧 Development

### Adding New Deobfuscation Pass

1. Create new file: `src/deobfuscate/your_pass.rs`
2. Implement function: `pub fn your_pass(tokens: Vec<Token>) -> Result<Vec<Token>, BeautifyError>`
3. Add to pipeline in `src/deobfuscate/mod.rs`
4. Add tests at bottom of your file
5. Run: `cargo test your_pass`

### JPL Compliance Rules

- ❌ Never use: `unwrap()`, `expect()`, `panic!()`
- ✅ Always use: `.checked_add()`, `.checked_sub()`, `.checked_mul()`
- ✅ Handle all errors with `Result` or `Option`
- ✅ Add `debug_assert!()` every 3-5 lines
- ✅ Use `#[cfg(debug_assertions)]` for debug-only code

### Trace Macros (Debug Only)

```rust
trace_inline!("inlined function '{}' at position {}", name, pos);
trace_prop!("converted obj[\"x\"] to obj.x at {}", pos);
trace_try!("removed try-catch at {}", pos);
trace_ternary!("simplified ternary at {}", pos);
```

All trace macros compile to nothing in release builds.

## 📝 Git Workflow

```bash
# View recent commits
git log --oneline -10

# Check status
git status

# View specific commit
git show <commit-hash>

# Compare files
git diff src/file.rs
```

## 🐛 Troubleshooting

### Build Fails
```bash
cargo clean && cargo build --release
```

### Tests Fail
```bash
# Run with output
cargo test -- --nocapture

# Run single test
cargo test test_name -- --nocapture
```

### Warnings About Unused Variables
- If variable is only used in trace macros, prefix with `_`
- Example: `let _pos = ...;` then use `_pos` in `trace_macro!(..., _pos)`

## 📚 Resources

- Session Summary: `SESSION_SUMMARY.md`
- Main README: `README.md`
- Test Examples: See bottom of each `src/deobfuscate/*.rs` file

