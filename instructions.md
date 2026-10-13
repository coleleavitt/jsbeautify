# Cross-Version JavaScript Alignment & Deobfuscation

## Goal

Port TypeScript cross-version alignment tools to Rust and integrate into jsbeautify to produce stable diffs between Bun-minified JavaScript bundle versions (e.g., Claude Code CLI across versions 2.1.88 → 2.1.96).

## Current Status

### Accomplished

1. **Cross-version alignment module** (`src/cross_version/`)
   - `mod.rs` - Main `CrossVersionAligner` with three-tier naming strategy
   - `ast_matcher.rs` - AST structure hashing, statement matching using `oxc_ast_visit::Visit`
   - `sourcemap_parser.rs` - VLQ decoder, sourcemap parsing, name extraction
   - `canonical_namer.rs` - Canonical naming infrastructure

2. **CLI integration**
   - Added `--sourcemap`, `--align-with`, `--align-output` flags
   - Fixed hashbang preservation in `esbuild_helper.rs`
   - Added `skip_annotations` option for alignment mode

3. **Performance**
   - O(n) single-pass replacement algorithm (was O(n²), timing out)
   - 14ms for 600k replacements

4. **Results achieved**
   - **94.7% statement match rate** between versions
   - **74.4% diff reduction** (637k → 163k lines)

### In Progress

1. **Three-tier naming strategy** (partially implemented in `mod.rs`):
   - Tier 1: Original names from sourcemaps (working)
   - Tier 2: Slot-based names `sN` using Bun alphabet (infrastructure exists, not wired in)
   - Tier 3: Statement-hash-based names `_rN` (working)

2. **Slot-based naming** - The following files exist but aren't integrated:
   - `src/ast_deobfuscate/deterministic_rename.rs` - Slot-based variable renamer
   - `src/ast_deobfuscate/bun_alphabet.rs` - Bun alphabet extraction and slot computation

## Architecture

### Bun Minification Insight

Bun uses a **frequency-based alphabet** for variable naming:
- **HEAD** (54 chars): Valid identifier start chars ordered by frequency
- **TAIL** (64 chars): Valid identifier continuation chars ordered by frequency
- Slot numbers represent frequency rank, which is stable across versions

```
name_to_slot("q") → 0  (most frequent single-char)
name_to_slot("K") → 1  (second most frequent)
slot_to_name(54) → "aa" (first two-char name)
```

### Bun Alphabet System (Detailed)

#### Default Alphabets

```rust
// HEAD: 54 chars for first position (no digits allowed)
pub const DEFAULT_HEAD: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";

// TAIL: 64 chars for subsequent positions (includes digits)
pub const DEFAULT_TAIL: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$";
```

#### Extracted Alphabet Example (from Claude Code bundle)

When Bun minifies, it reorders these alphabets by frequency. An extracted alphabet might look like:

```rust
// Most frequent chars first
head = "qKzYOAwjHJMXuPWfaDZsvGLkbTVgyxEShNRCdnpmIt$iBreFcUQol"
tail = "f68o1ns7q4drKte53_A9$zYO0y2pwjHDMiPaJXkRWvZThNmGLbclESIuVxCgBFQU"
```

#### Slot Computation Algorithm

```rust
// slot_to_name: Convert slot number to minified name
pub fn slot_to_name(&self, mut slot: usize) -> String {
    let mut name = String::new();
    
    // First character from HEAD (base-54)
    let first_idx = slot % 54;
    name.push(head_chars[first_idx]);
    slot /= 54;
    
    // Subsequent characters from TAIL (base-64)
    while slot > 0 {
        slot -= 1;
        let idx = slot % 64;
        name.push(tail_chars[idx]);
        slot /= 64;
    }
    name
}

// name_to_slot: Convert minified name back to slot number
pub fn name_to_slot(&self, name: &str) -> Option<usize> {
    let chars: Vec<char> = name.chars().collect();
    
    // First char uses HEAD (base-54)
    let mut slot = head_to_pos[chars[0]];
    
    // Subsequent chars use TAIL (base-64)
    let mut multiplier = 54;
    for c in &chars[1..] {
        let pos = tail_to_pos[c];
        slot += (pos + 1) * multiplier;
        multiplier *= 64;
    }
    Some(slot)
}
```

#### Slot Number Examples

| Slot | Name (default) | Name (extracted) |
|------|----------------|------------------|
| 0 | `a` | `q` |
| 1 | `b` | `K` |
| 25 | `z` | `o` |
| 26 | `A` | `l` |
| 53 | `$` | (last single-char) |
| 54 | `aa` | `qf` |
| 55 | `ba` | `Kf` |
| 3510 | `aaa` | (first 3-char) |

#### Alphabet Extraction Process

The `AlphabetExtractor` analyzes minified source to determine the actual alphabet ordering:

```rust
pub struct AlphabetExtractor {
    single_char_freq: FxHashMap<char, usize>,  // Determines HEAD ordering
    second_char_freq: FxHashMap<char, usize>,  // Determines TAIL ordering
}

impl AlphabetExtractor {
    pub fn record_identifier(&mut self, name: &str) {
        // Single-char identifiers → HEAD frequency
        if len == 1 && is_valid_head_char(c) {
            *self.single_char_freq.entry(c).or_insert(0) += 1;
        }
        // Two-char identifiers → TAIL frequency (second char)
        if len == 2 && is_valid_tail_char(c) {
            *self.second_char_freq.entry(chars[1]).or_insert(0) += 1;
        }
    }
    
    pub fn build_alphabet(&self) -> BunAlphabet {
        // Sort by frequency descending, fill missing chars from defaults
        let head = build_sorted_alphabet(&self.single_char_freq, false);
        let tail = build_sorted_alphabet(&self.second_char_freq, true);
        BunAlphabet::new(head, tail)
    }
}
```

#### Why Slots Are Stable Across Versions

The key insight: **slot numbers represent semantic frequency rank, not absolute character position**.

When comparing two versions:
1. Extract alphabet from each version independently
2. Convert each minified name to its slot number using that version's alphabet
3. The slot numbers will match for the same logical variable

```
Version 2.1.88:
  Alphabet: "qKzYOA..." (q is most frequent)
  Variable "q" → slot 0
  
Version 2.1.96:
  Alphabet: "qKzYOA..." (same ordering, q still most frequent)
  Variable "q" → slot 0
  
Result: Same slot = same variable across versions!
```

#### Current Integration Status

The slot-based naming is **implemented but not wired into cross-version alignment**:

```rust
// In src/ast_deobfuscate/mod.rs line 109:
deterministic_renamer: DeterministicRenamer::new(),  // Instantiated but NEVER CALLED

// The deobfuscation pipeline uses variable_renamer with _rN naming instead
```

To integrate, modify `cross_version/mod.rs`:

```rust
use crate::ast_deobfuscate::bun_alphabet::{extract_alphabet_from_source, BunAlphabet};

impl CrossVersionAligner {
    pub fn align_sources(&self, source_code: &str, target_code: &str) -> (...) {
        // Extract alphabets from both versions
        let source_alphabet = extract_alphabet_from_source(source_code);
        let target_alphabet = extract_alphabet_from_source(target_code);
        
        // In the naming loop:
        let canonical = if let Some(stable) = self.stable_names.get(&src_id.name) {
            stable.clone()  // Tier 1: Sourcemap
        } else if let Some(slot) = source_alphabet.name_to_slot(&src_id.name) {
            format!("s{slot}")  // Tier 2: Slot-based
        } else {
            format!("_r{canonical_counter}")  // Tier 3: Fallback
        };
    }
}
```

### Three-Tier Naming Strategy

```rust
for src_id in &source_stmt.identifiers {
    let canonical = if let Some(stable) = self.stable_names.get(&src_id.name) {
        // Tier 1: Sourcemap has original name
        stable.clone()
    } else if let Some(slot) = source_alphabet.name_to_slot(&src_id.name) {
        // Tier 2: Slot-based name (semantic, stable)
        format!("s{slot}")
    } else {
        // Tier 3: Statement-hash-based fallback
        format!("_r{canonical_counter}")
    };
}
```

### Sourcemap Name Extraction (How stable_names is Built)

The sourcemap provides mappings from minified positions to original positions. We use this to recover original variable names.

#### Sourcemap Structure

```json
{
  "version": 3,
  "sources": ["src/index.ts", "src/utils.ts", ...],
  "sourcesContent": ["const foo = ...", "export function bar...", ...],
  "names": [],  // Often EMPTY in Bun sourcemaps!
  "mappings": "AAAA,SAAS,CAAC,CAAC,CAAC,CAAC..."  // VLQ-encoded
}
```

**Key insight**: Bun sourcemaps have an **empty `names` array** but include full `sourcesContent`. We extract names by looking up identifiers at mapped positions.

#### VLQ Decoding

The `mappings` string is VLQ (Variable-Length Quantity) encoded:
- `;` separates lines in the generated (minified) file
- `,` separates segments within a line
- Each segment contains 4-5 base64-encoded values:
  1. Generated column (relative to previous)
  2. Source file index (relative)
  3. Original line (relative)
  4. Original column (relative)
  5. Name index (optional, often missing)

```rust
fn decode_vlq(input: &str) -> Vec<i64> {
    const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                                  abcdefghijklmnopqrstuvwxyz\
                                  0123456789+/";
    // Decode continuation bits, sign bit handling...
}
```

Example: `"AAAA"` decodes to `[0, 0, 0, 0]` (all zeros)

#### Name Extraction Algorithm

```rust
pub fn extract_names(&self, sourcemap_json: &str, bundle_source: &str) -> Vec<NameMapping> {
    // 1. Parse sourcemap JSON
    let raw: RawSourcemap = serde_json::from_str(sourcemap_json)?;
    
    // 2. Build index of identifiers in minified bundle: (line, col) -> name
    let bundle_identifiers = extract_identifiers_with_positions(bundle_source);
    
    // 3. Decode VLQ mappings
    let decoded = decode_mappings(&raw.mappings);
    // Each entry: (gen_line, gen_col, source_idx, orig_line, orig_col)
    
    // 4. For each mapping, look up both names
    for (min_line, min_col, src_idx, orig_line, orig_col) in decoded {
        // Get original source content
        let source_content = raw.sources_content[src_idx];
        
        // Look up identifier at original position
        let original_name = get_identifier_at(source_content, orig_line, orig_col);
        
        // Look up identifier at minified position  
        let minified_name = bundle_identifiers.get(&(min_line, min_col));
        
        // Create mapping
        mappings.push(NameMapping {
            minified_name,      // e.g., "q"
            original_name,      // e.g., "config"
            source_file,        // e.g., "src/config.ts"
            original_line,
            original_column,
            minified_line,
            minified_column,
        });
    }
}
```

#### Filtering to Stable Names

Not all mappings are usable. We filter to create `stable_names`:

```rust
pub fn load_sourcemap(&mut self, sourcemap_json: &str, bundle_source: &str) -> Result<usize> {
    let mappings = parser.extract_names(sourcemap_json, bundle_source)?;
    
    // Group by minified name
    let mut name_index: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for mapping in &mappings {
        name_index
            .entry(mapping.minified_name.clone())
            .or_default()
            .push(mapping.original_name.clone());
    }
    
    // Only keep unambiguous mappings (one minified name → one original name)
    for (minified, originals) in name_index {
        if originals.len() == 1 {
            let original = &originals[0];
            if !is_reserved(original) {  // Skip "undefined", "console", etc.
                self.stable_names.insert(minified, original.clone());
            }
        }
    }
}
```

#### Why Only 6,148 Names Recovered (1.1%)

The low recovery rate is because:

1. **Ambiguous mappings**: Same minified name (e.g., `q`) maps to different original names in different scopes
2. **Position misalignment**: VLQ positions don't always land exactly on identifier starts
3. **Inlined code**: Some identifiers in the bundle don't exist in original sources
4. **Reserved words filtered**: Common names like `undefined`, `console`, `process` are skipped

#### Improving Name Recovery (Future Work)

1. **Scope-aware mapping**: Track which scope each mapping belongs to
2. **Fuzzy position matching**: Look for nearby identifiers if exact position misses
3. **Name frequency voting**: If `q` maps to `config` 100x and `data` 2x, pick `config`
4. **Cross-reference with AST**: Use AST node types to validate mappings

## Test Commands

```bash
# Basic beautify with deobfuscation
/home/cole/RustProjects/active/jsbeautify/target/release/jsbeautify input.js -d -o output.js

# Cross-version alignment
/home/cole/RustProjects/active/jsbeautify/target/release/jsbeautify \
  cli.2.1.88.js -d \
  --sourcemap cli.js.map \
  --align-with cli.2.1.96.js \
  -o /tmp/aligned-88.js \
  --align-output /tmp/aligned-96.js

# Then diff
diff /tmp/aligned-88.js /tmp/aligned-96.js | wc -l
```

## Test Data Locations

- `/home/cole/VulnerabilityResearch/anthropic/cli.js.map` - 57MB sourcemap for 2.1.88
- `/home/cole/VulnerabilityResearch/anthropic/cli.2.1.88.js` - Source bundle
- `/home/cole/VulnerabilityResearch/anthropic/cli.2.1.96.js` - Target bundle

## Research Papers (Downloaded)

Located in `/home/cole/VulnerabilityResearch/anthropic/research/`:

| Paper | Technique | Accuracy |
|-------|-----------|----------|
| `jsnice-2015.pdf` | CRF-based type inference + naming | Baseline |
| `jsneat-2019.pdf` | Information Retrieval (IR) | 69.1% |
| `jsnaughty-2017.pdf` | Statistical Machine Translation | ~50% |
| `context2name-2018.pdf` | RNN deep learning | 47.5% |
| `dire-2019.pdf` | Neural encoder-decoder | 74.3% |

### Key Techniques

1. **JSNice** (ETH Zürich) - Conditional Random Fields for type inference
2. **JSNeat** - IR-based search in large JS corpus using usage contexts
3. **JSNaughty** - SMT (Moses) treating minification as translation
4. **Context2Name** - RNN learning from surrounding code context
5. **DIRE** - Neural approach for decompiled identifier naming

## Deobfuscation Tools (Cloned)

Located in `/home/cole/VulnerabilityResearch/anthropic/tools/deobfuscation-research/`:

| Tool | Language | Purpose | Status |
|------|----------|---------|--------|
| `webcrack` | TypeScript | Deobfuscate obfuscator.io, unpack webpack | Built |
| `wakaru` | TypeScript | JS decompiler for webpack/browserify | Needs pnpm |
| `humanify` | TypeScript | LLM-based variable naming | Native build issues |
| `restringer` | TypeScript | JS deobfuscator for obfuscator.io | Native build issues |
| `jsneat` | Java | IR-based name recovery | Needs Java setup |
| `jsNaughty` | Python/Docker | SMT-based deobfuscation | Needs Moses/Docker |
| `dire` | Python | Neural identifier naming | Deprecated, use CMUSTRUDEL |
| `sourcemapper` | Go | Extract files from sourcemaps | Ready |

### Tool Usage

```bash
# webcrack
cd tools/deobfuscation-research/webcrack
pnpm install && pnpm run build
node packages/webcrack/dist/cli.js input.js -o output/

# sourcemapper (Go)
cd tools/deobfuscation-research/sourcemapper
go build
./sourcemapper -url http://example.com/bundle.js.map -output ./extracted/
```

## TypeScript Reference Implementation

`/home/cole/VulnerabilityResearch/anthropic/tools/ultimate-align.ts` contains the original TypeScript implementation that was ported to Rust.

## Key Files in jsbeautify

```
src/
├── cross_version/
│   ├── mod.rs              # CrossVersionAligner, AlignConfig, AlignmentStats
│   ├── ast_matcher.rs      # StatementMatcher, StatementInfo, structure hashing
│   ├── sourcemap_parser.rs # VLQ decoding, SourcemapParser, NameMapping
│   └── canonical_namer.rs  # CanonicalNamer (placeholder)
├── ast_deobfuscate/
│   ├── mod.rs              # Main deobfuscator pipeline
│   ├── bun_alphabet.rs     # BunAlphabet, AlphabetExtractor, slot computation
│   ├── deterministic_rename.rs # DeterministicRenamer (NOT INTEGRATED)
│   └── esbuild_helper.rs   # Hashbang fix, module annotations
├── options.rs              # Added skip_annotations option
└── bin/jsbeautify.rs       # CLI with alignment flags
```

## Next Steps

1. **Integrate slot-based naming**: Wire `bun_alphabet.rs` into `cross_version/mod.rs`
2. **Improve sourcemap extraction**: Currently only 6,148 names recovered (1.1% of 564k identifiers)
3. **Test deobfuscation tools**: Set up and benchmark webcrack, wakaru on Claude bundles
4. **Consider LLM integration**: humanify approach for semantic naming as post-processing

## Constraints from User

- "don't preprocess right idk check oxc or how does the typescript code it and edit oxc if needed or our jsbeautify right in a sense correctly right"
- "don't use a simpler approach use the proper approach right"
- "implement all of it properly please thank you"

## Git History

```
b0252e2 feat(cross_version): add cross-version alignment
```

## Performance Notes

- Statement extraction: ~2-3s for 600k line file
- Hash index building: ~100ms
- Statement matching: ~2s
- Replacement application: ~14ms (after O(n) fix)
- Total alignment: ~5-6s for two 600k line files
