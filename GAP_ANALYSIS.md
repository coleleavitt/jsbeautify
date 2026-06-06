# Pipeline Gap Analysis — js-beautify-rs vs. the Bun-bundled Claude Code `cli.js`

> Date: 2026-06-05 · Target: `cli.extracted.js` 2.1.165 (16.18 MB, Bun `// @bun @bytecode @bun-cjs` standalone CJS bundle)
> Method: static multi-agent audit (35 agents — pipeline inventory × Bun bundler/minifier/printer ground-truth × empirical bundle sampling × research roadmap, then adversarial verification of every candidate) **plus** a dynamic run of `target/release/jsbeautify -d` on the real bundle. Every claim below was checked against both the bundle bytes and the pipeline source.

---

## 1. Executive summary

The deobfuscator is **excellent at Bun module-level *name* recovery** but has three classes of problems on its primary target:

1. **It emits invalid JavaScript.** The deobfuscated output of 2.1.165 does **not** pass `node --check`. Three *independent* bugs each corrupt the output:
   - unescaped backtick / `${` in reconstructed template literals (`dyn_concat_resolver`),
   - duplicate top-level declarations from string-keyed (non-scope-aware) renaming (`bun_export_renamer`),
   - dangling free variables from renaming a parameter's declaration but not its body references (`param_heuristic`, 2,578×).
2. **The core Bun structural reversal is missing or dead.** All **5,610** lazy module wrappers (`__commonJS`/`__esm`) stay trapped in closures — the `bun_commonjs_inliner`/`bun_esm_inliner` passes promised in the code **do not exist**. The whole 16 MB file remains nested inside one `(function(exports,require,module,…){…})` scope that is never flattened. `DeterministicRenamer` is constructed but **never traversed** (dead code).
3. **Function-local identifiers stay minified.** ~268k bare `H/$/q/K/_` tokens, 51k single-char `let/const`, 16k short-named functions survive, because `variable_rename` only matches the `_0x…` (javascript-obfuscator.io) convention, which Bun never produces.

Meanwhile ~60+ passes target obfuscations Bun never emits (string-array/CFF/RC4/Akamai/`_0x`), several of which are **no-op stubs**, and ~9 analyzer files **never compile** (no `pub mod`).

**Highest-leverage work, in order:** (T0) make the output valid JS + add a re-parse validity gate; (T1) flatten the CJS wrapper and inline the 5,610 module factories; (T2) make all renamers scope-aware (`SymbolId`, not string match) and add local renaming; (T3) the cheap, safe minify-syntax reversals (`typeof x>"u"`, `1/0`); (T4) robustness (3.25 GB RSS) and dead-code cleanup.

---

## 2. What already works well (keep / build on)

Measured from the live run on 2.1.165:

| Capability | Phase | Result on real bundle |
|---|---|---|
| Bun runtime-helper detection + rename | 0.9 | 13 helpers + **8,418** refs → `__commonJS`,`__esm`,`__toESM`,`__export`,`__copyProps`,`__toCommonJS`,`__defProp`,… |
| `__export({k:()=>v})` export-name recovery | 0.95 | **4,431** maps → **32,481** identifiers renamed (4,687 decls + 27,794 refs) — the single biggest readability win |
| Class/displayName recovery | 0.97 | 50 names → 423 idents |
| Statement de-minification | 14–15 | 10,592 sequence-splits, 4,273 return-splits, 9,556 if-body comma splits, 17,572 multi-var splits |
| Boolean / void normalization | 11 | `!0/!1` → 0 leftover; `void 0` → 3 leftover |
| Dead code/var elimination | 7,8,10.5,… | ~13k dead vars removed across sweeps |

These are correct and end-to-end verified (the `__export` recovery was explicitly probed for robustness and ruled solid). The gaps below are **net-new** on top of this.

---

## 3. Ranked gaps

Severity key: **🔴 critical** (output correctness or core mission), **🟠 high**, **🟡 medium**, **⚪ low/partial**.

### Tier 0 — Correctness: the output must be valid JavaScript

#### 🔴 C1 — Template-literal reconstruction emits unescaped `` ` `` / `${` (dynamic-only finding)
- **Where:** `src/ast_deobfuscate/dyn_concat_resolver.rs:253` and `:270` — `let raw = ctx.ast.str(&pending_text);` sets the `TemplateElement.raw` to the **unescaped** cooked text.
- **Evidence:** `node --check /tmp/cli.165.deob.js` fails at L9941:
  `hc9 = `[^\s-]{0,4}-----BEGIN…PRIVATE KEY…-----['"`]?`;` — the literal `` ` `` inside the regex char-class `['"`]` terminates the template early.
- **Root cause:** when the resolver inlines a string value containing `` ` ``, `${`, `\`, or a newline into a template quasi, `raw` must be the *escaped* form; it is currently identical to `cooked`.
- **Fix:** when emitting the quasi, either (a) **prefer a `StringLiteral`** when the merged run has no interpolations (let oxc codegen handle quote escaping), or (b) for genuine templates escape `raw`: `` ` ``→`` \` ``, `${`→`\${`, `\`→`\\`. Add a unit test with a backtick-containing payload.

#### 🔴 C2 — Duplicate declarations from string-keyed renaming → redeclaration errors (= GAP-13 manifestation)
- **Where:** `src/ast_deobfuscate/bun_export_renamer.rs:210-215` (and `bun_runtime.rs:336-341`, `class_name_renamer`) rewrite references by **raw string equality program-wide** and key the rename map by mangled name only.
- **Evidence:** Phase 11 reports `Identifier 'ZodType'/'Client'/'Command'/'NoOpLogger' has already been declared`. Output contains both `class ZodType` (L13064) **and** `var ZodType` (L25914); `Command`×6, `NoOpLogger`×6. Realized collision: `__export(nm$,{logEvent:()=>c,…})` makes the map `c → logEvent`, but `c` occurs **3,414×** as an unrelated token (and 28× as `(c)=>` params) across sibling scopes — all rewritten to `logEvent`.
- **Root cause:** Bun reuses 1–2 char names across non-overlapping scopes (slot reuse); a name-keyed, scope-blind rewrite collapses distinct symbols. (Note: the runtime *helper* names `m/V/o` do **not** actually collide in real AST positions here — `ExportNameRenamer` is the real failure, `ClassNameRenamer` shares the risk.)
- **Fix:** the scope-aware rename infrastructure in **T2/GAP-13** below. As an interim guard, refuse to map mangled names ≤2 chars and bound export/class renames to the declaring module scope.

#### 🔴 C3 — `ParamHeuristicRenamer` renames the parameter declaration but **not** its body uses (= GAP-09)
- **Where:** `src/ast_deobfuscate/param_heuristic.rs:108-112` (explicit TODO); wired at `mod.rs:519-529` (Phase 0.98).
- **Evidence:** live run renamed **2,578** params; output has e.g. `function setOriginalCwd(input){ l$.originalCwd = J1$(H); … }` — the param is `input` but the body still references the now-undeclared `H`. Reproduced in isolation: `function processData(H,$){…}` → `function processData(key,value){ console.log(H); return H+$; }`.
- **Root cause:** the pass mutates only the `BindingIdentifier`, leaving every `IdentifierReference` dangling; `infer_param_name` is purely positional (ignores `_body`).
- **Fix:** make it scope-aware — after choosing a name, rewrite the references that resolve to the param's `SymbolId` (only that binding, respecting inner shadowing). Derive names from body usage (member access, call-arg position). **Until fixed, disable the pass** — it actively corrupts output.

#### 🔴 C0 — Add an output-validity gate (cross-cutting, prerequisite)
- After codegen, **re-parse the output** with oxc; if it has parse/redeclaration errors, log them prominently (and ideally bisect which phase introduced them). C1–C3 would all have been caught by this. The research corpus reinforces this: JSIMPLIFIER reports renaming as its top source of semantic errors → any aggressive transform should be gated by an equivalence/validity check, and any future LLM-assisted renaming must be name-only.

---

### Tier 1 — Core Bun structural reversal (the actual mission)

#### 🔴 T1a — Flatten the top-level `@bun-cjs` wrapper IIFE (= GAP-03)
- **Where:** no pass does this. `bun_runtime.rs:390 unwrap_cjs_iife` only *descends* to find helpers; `iife_unwrap.rs` only handles zero-arg **arrow** IIFEs.
- **Evidence:** the entire file is one `(function(exports,require,module,__filename,__dirname){ … lzz(); })` — a parenthesized, non-invoked `FunctionExpression` spanning all 16.18 MB (closes `…lzz();})` at EOF). Every statement lives one scope deep.
- **Fix:** new pass `FlattenBunCjsWrapper` (oxc `Traverse`, run as **Phase 0.91**, right after helper rename). When `program.body` is a single `ExpressionStatement` matching `unwrap_cjs_iife`'s shape (paren `FunctionExpression` with ≥3 of the 5 CJS magic params), splice the body to top level; preserve the trailing entry call (`lzz();`); the 5 params become runtime-provided free identifiers (fine for a readability tool). Run before all control-flow/IIFE passes so they see flat top-level code.

#### 🔴 T1b — Inline the 5,610 lazy module wrappers `__commonJS`/`__esm` (= GAP-01)
- **Where:** the `bun_runtime.rs:35` docstring **promises** `bun_commonjs_inliner` / `bun_esm_inliner` — **neither exists** in the tree or in `mod.rs`. Current coverage is cosmetic `// Bun Module:` banners only.
- **Evidence:** **1,591** `__commonJS` (`=o((`) + **4,019** `__esm` (`=V((`) = **5,610** module factories. E.g. `YDq=o((izz,AB9)=>{AB9.exports=require("/$bunfs/root/image-processor.node")})` and `wDq=V(()=>{MDq=fB9})`. Bun basis: `runtime.js:105`/`:316`; `LinkerContext.zig` `createWrapperForFile`.
- **Fix:** new **Phase 0.99** two-part pass (after name recovery, before generic obfuscator passes):
  - *Collector* (`enter_variable_declarator`): match declarators whose init is a `CallExpression` to the canonical `__commonJS`/`__esm` `SymbolId`; record handle `SymbolId`, kind, body, exports object.
  - *Rewriter:* for `__esm`, hoist the body into a named module block and replace `H()` init calls with the exports ref; for `__commonJS`, lower `(exports,module)=>{…}` into a namespace, rewrite `module.exports=`/`exports.x=` to members, and replace `require_H()` / `__toESM(require_H())` call sites with the namespace ref.
  - **Preserve boundaries** (one module = one named block) rather than flattening all 5,610 into one scope. Requires the scope-aware infra (T2).

#### 🔴 T1c — Reconstruct module boundaries: handle-var renaming + dependency graph (= GAP-02)
- **Where:** the real splitter `bun_extract.rs` (`find_module_wrappers`/`split_bundle`) is wired **only** to the `--bun-extract` CLI flow (`jsbeautify.rs:1322`, `cross_version/mod.rs:131`), **not** to `AstDeobfuscator::deobfuscate` (`beautifier/mod.rs:30-34`). (Correction to an earlier overstatement: `bun_extract` is *not* dead code — it's just not on the deobfuscate path.)
- **Evidence:** module-handle vars form a require/init graph that is never reconstructed: `DDq=V(()=>{a9$();…})`, `a9$=V(()=>{EmH();…})`. 541 `__export` (`w$(`) sites.
- **Fix:** alongside T1b at Phase 0.99: (1) rename each module-handle `SymbolId` to a readable `require_<n>`/`init_<n>` (derived from path comment / `__export` default target / first `this.name`); (2) record `require_H()`/`H()` edges into `ctx.state` as a dependency graph; (3) feed the Phase 20 annotator the real module name + import list instead of the `\(\s*\)\s*=>` line regex.

---

### Tier 2 — Naming (the dominant residual after structure)

#### 🟠 T2a — Scope-aware identifier-rewrite infrastructure (= GAP-13)
- The fix for C2 and a prerequisite for T1b/T2b. Add a shared utility: build oxc `SemanticBuilder` scoping, map (mangled name + decl site) → `SymbolId`, rewrite **only** `scoping.get_resolved_references(symbol)`, remove declarations by `SymbolId`. Refactor `BunRuntimeRenamer`, `ExportNameRenamer`, `ClassNameRenamer`, `VariableRenamer`, `ParamHeuristicRenamer`, and the T1b inliner onto it. (The scoping is *already built* before each pass at `mod.rs:441/448/465/…` but the renamers ignore it.)

#### 🟠 T2b — Rename Bun minified locals; `DeterministicRenamer` is dead (= GAP-04)
- **Where:** `DeterministicRenamer` is declared (`mod.rs:181`), constructed (`mod.rs:221`), but **never traversed** (zero `traverse_mut_with_ctx(&mut self.deterministic_renamer …)`; its logic runs only in `#[cfg(test)]`). `VariableRenamer.should_rename` (`variable_rename.rs:32-43`) matches only `_0x`/`_<hex>`, so Bun slot names never match.
- **Evidence:** `let $=`×5,004, `let q=`×3,826, `let K=`×2,972, `let _=`×2,400, `let A=`×2,132, `let H=`×1,685, …; `${H}`×2,062 interpolations. (~268k bare single-char tokens overall.)
- **Reality check:** per Bun's `renamer.zig` (`assignNamesByFrequency`), these names are slot/frequency-derived and carry **zero** original-name info. So renaming yields **stable, readable slots** (`s0,s1,…` or usage-heuristic names like `args`/`idx`/`result`), not semantic recovery. Still valuable: readability + deterministic cross-version diffs (the project's `--align-with` goal).
- **Fix:** wire a scope-aware heuristic renamer at Phase 12 on top of T2a — derive names from usage signals (recovered export name, call target, string-literal args, member-access shape, param/loop-var role) with stable slot fallback; apply Bun's reserved-word set + in-scope global-shadow avoidance. Either repair `DeterministicRenamer`'s wiring or delete it (it currently creates a false "handled" impression).

---

### Tier 3 — Minify-syntax reversals (cheap, safe, high per-site value)

#### 🟠 T3a — `typeof x > "u"` / `< "u"` → `=== "undefined"` / `!== "undefined"` (= GAP-05)
- **Evidence:** **391** sites (240 `<"u"` + 151 `>"u"`) vs only 3 surviving literal `==="undefined"`. E.g. `if(typeof process<"u"&&…`, `typeof window<"u"`, `typeof Deno<"u"`. No pass reverses it (`grep '"u"' src/` = 0). Bun basis: `visit_binary.zig tryOptimizeTypeofUndefined`.
- **Fix:** new `TypeofUndefinedNormalizer` (exit_binary_expression, ~Phase 6–8): `typeof e > "u"` → `typeof e === "undefined"`; `<` → `!==`. Exact, near-zero false-positive.

#### 🟡 T3b — `1/0` → `Infinity`, `-1/0` → `-Infinity`, `0/0` → `NaN` (= GAP-11)
- **Evidence:** **196** `1/0` sites; decisive: `case"Infinity":return 1/0;case"-Infinity":return-1/0`, `maxSockets:1/0`, `maxBodyLength=1/0`. `ConstantFolder` is i64-only and declines div-by-zero (`constant_folding.rs:38-43`), so these survive.
- **Fix:** small `NumericIdiomFolder` emitting an `IdentifierReference` (not a numeric literal); restrict to literal-numerator / zero-literal-denominator.

#### 🟡 T3c — Distinguish `__reExport` from `__copyProps` (= GAP-06)
- **Evidence:** Bun's 3-arg `__reExport` (`Nq=(H,$,q)=>{var K=OF8($);for(let _ of K)if(!Gu$.call(H,_)&&_!=="default")…;if(q){…return q}}`) is matched by `is_copyprops_helper` and mislabeled `__copyProps`. Used as re-export 35× (≥2 args). A *real* 4-param `__copyProps` (`bu_`) also exists in a nested module but `is_copyprops_helper` requires exactly 3 params, so it's missed entirely.
- **Fix:** add `BunHelper::ReExport` + `is_reexport_helper` ordered **before** `is_copyprops_helper` (distinguishing signal: hardcoded `!=="default"` + conditional second-target loop returning the 3rd param). Tighten `is_copyprops_helper` to recognize the genuine 4-param form. A T1c consumer can then annotate `export * from <mod>`.

---

### Tier 4 — Robustness & code health

#### 🟠 T4a — Memory: 3.25 GB RSS for a 16 MB input (dynamic-only finding)
- **Evidence:** live run peaked at **3,246,316 KB** RSS; 21.3 s user / 28.2 s wall. The project root holds **935 MB / 54 MB / 28 MB core dumps** — consistent with OOM on larger/parallel inputs.
- **Root cause:** the pipeline runs **~30 full `SemanticBuilder::new().build(&program)` rebuilds** (one+ per phase), each producing fresh scoping over the entire ~20 MB AST, plus per-phase `into_scoping()` allocations in the bump allocator that are never reclaimed mid-run.
- **Fix:** (1) reuse/incrementally update scoping where a pass doesn't invalidate it instead of rebuilding every phase; (2) batch independent passes that share a scoping snapshot under one build; (3) consider per-module processing after T1b (process one inlined module subtree at a time). Even halving the rebuilds materially cuts peak RSS.

#### 🟠 T4b — Dead code, uncompiled files, and no-op stubs (= GAP-16)
- **Hard-verified, fix regardless of framing:**
  - **~9 analyzer files never compile** (no `pub mod` anywhere, zero refs): `webpack_module_resolver.rs`, `webpack_runtime_decoder.rs`, `transmit_sdk_decoder.rs`, `angular_decompiler.rs`, `angular_metadata.rs`, `bundle_diff.rs`, plus the `ngrx`/`mfe`/`launchdarkly`/`http_interceptor` extractors. → wire or delete.
  - **4 pipeline no-op stubs** that run but only increment counters: `switch_true_converter.rs` (self-documented no-op, `mod.rs:994`), `json_parse_eval.rs` (`:1004`), `optional_chaining_normalizer.rs` (`:1014`), `nullish_coalescing_simplifier.rs` (`:1024`). → finish or delete.
  - `DeterministicRenamer` unwired (see T2b).
- **Prioritization:** ~60+ obfuscator-family passes (string-array/CFF/RC4/XOR/Akamai/`_0x`) are **inert** on this Bun target — verified: `_0x[0-9a-f]{4,}`=0 hits, `eval(`=only 2 in 16 MB, 0 string-array decoder IIFEs. They're cheap (no-match traversals) and may serve other inputs, so **gate them behind a one-time signature sniff** rather than delete, and promote an explicit Bun-first track (see §4). *(Corrections to the original synthesis: `bun_extract` is wired via CLI/cross_version; renaming is not entirely absent — params/exports/classes/helpers are renamed.)*

---

### Tier 5 — Low priority / partially handled (do not prioritize)

- **⚪ GAP-12 — comma/sequence completeness:** *mostly handled.* The headline 3,632+ React-Compiler memo chains ARE split (by `IfBodyCommaSplitter`, Phase 14.7 — verified on the live output). Residual: the Phase-21 `expand_arrow_sequences` is a fragile post-codegen regex that leaves **2-element** arrow bodies (`()=>(QS6(),y6(x))`, ~37 of 42 sites) untouched. If pursued, replace the regex with a small AST pass that converts arrow-expression-body `SequenceExpression`s to block form. Tiny site count → low value.
- **⚪ GAP-14 — `__toESM`/`__toCommonJS` interop annotation:** *partially handled* (helpers already renamed, which surfaces the transition by name). 120 `__toESM(require("fs"…))`, 2,012 node-mode `m(X(),1)`, and `(init(),y6(ns))` interop sequences exist, but full `import * as` rewriting is semantically risky on concatenated requires. At most add a per-call-site annotation comment in the T1c consumer. (The literal `module.exports=y6(X)` shape has 0 hits — it appears as `(init(),y6(ns))`.)

---

## 4. Ruled out (verified **not** gaps)

| Candidate | Verdict | Why |
|---|---|---|
| `__export` recovery robustness (callee forms, setter trap, map variants) | already handled | Works end-to-end on this exact bundle (32,481 renames). |
| `__name(fn,"Orig")` call-site name recovery | **not applicable** | Claude is built **without** keep-names — **0** `__name(` calls in the raw bundle; `__name` is tree-shaken out. (Generic technique; inert here.) |
| String-escape decoding (`\xNN`,`\uNNNN`,`\u{…}`) | already handled | oxc parse→codegen round-trip normalizes escapes; no pass needed. |
| Newer export-setter helpers (`KB9` identity, `_B9` setter binder) | already handled | `esbuild_helper.rs` detects all three via `is_identity_pattern`. |

---

## 5. Recommended pipeline reordering (Bun-first track)

```
Phase 0.0  Encrypted-eval (unchanged)
Phase 0.4  Window-alias (unchanged)
Phase 0.5  [GATE] obfuscator-family sniff: skip Akamai/string-array/CFF/RC4 unless signatures present
Phase 0.9  Bun helper rename  + [T3c] ReExport variant
Phase 0.91 [T1a] Flatten top-level @bun-cjs wrapper IIFE          ← NEW
Phase 0.95 __export name recovery  (now scope-aware via T2a)
Phase 0.97 Class/displayName       (scope-aware)
Phase 0.98 [C3] Param rename — FIX body refs or DISABLE
Phase 0.99 [T1b/T1c] Module-wrapper inline + handle rename + dep graph  ← NEW
Phase 6-8  expression cluster + [T3a] typeof>"u" + [T3b] 1/0→Infinity   ← NEW rules
Phase 12   [T2b] scope-aware local renamer (wire DeterministicRenamer or replace)
…
Phase 19.6 [C0] re-parse output; report parse/redeclaration errors      ← NEW gate
Phase 20   annotate from the real module graph (not the line regex)
```

---

## 6. Methodology note

Static findings came from 35 subagents (pipeline inventory, Bun `js_printer`/`p.zig`/`renamer.zig`/`runtime.js`/`bundle_v2.zig`/`LinkerContext.zig` ground-truth, empirical sampling of `cli.extracted.js`, and the `research/cff-v4` corpus), each candidate gap then independently verified against the bundle bytes and the pipeline source. Dynamic findings (C1, T4a, and confirmation that the output is invalid JS) came from running `target/release/jsbeautify -d` on the real 2.1.165 bundle and inspecting the phase counts + output. Where the original synthesis overstated a claim, the verified/corrected fact is used here.

## 7. oxc API verification (fixes are buildable on the pinned 0.124)

Verified against a local oxc checkout (`~/RustProjects/forks/oxc`; fork is 0.134, the `crates_v0.124.0` tag this project pins is present; the APIs below are stable across that range):

- **C1 (template escaping):** `oxc_codegen` prints `TemplateElement.value.raw` **verbatim** (`crates/oxc_codegen/src/gen.rs:3958` → `p.print_str(quasi.value.raw.as_str())`). Confirms the fix must escape `` ` ``/`${`/`\` *before* storing `raw` in `dyn_concat_resolver.rs:253/270`, or emit a `StringLiteral` and let codegen escape.
- **C2/C3/T2a (scope-aware rename):** supported — `Scoping::get_resolved_references(symbol_id)` and `symbol_references(...)` (`crates/oxc_semantic/src/scoping.rs:551`, `lib.rs:245`), `TraverseCtx::scoping()/scoping_mut()`, and `BoundIdentifier{ symbol_id }` give per-`SymbolId` reference rewriting. The pipeline already builds `SemanticBuilder` scoping before each phase; the renamers simply ignore it.
- **Angular pass currency:** the (dead) `angular_decompiler.rs` already matches **current** Ivy instructions incl. v17/18 control flow (`ɵɵconditional`/`ɵɵdefer`/`ɵɵdeclareLet`) — so wiring it is higher value than rewriting it. (Tracked in the separate framework-extraction roadmap.)
