# Framework-Extraction Gap Analysis — js-beautify-rs

> Date: 2026-06-05 · Companion to `GAP_ANALYSIS.md` (which covers the core Bun/oxc deobfuscation pipeline).
> Scope: js-beautify-rs as a **static recon tool for minified production web-app bundles** — what framework-specific structure (routes, component trees, store slices, API/GraphQL endpoints, feature-flag keys, module/chunk graphs, telemetry DSNs) it can recover, and what it's missing.
> Method: 18 frameworks were sparse-cloned at **current** versions into `~/RustProjects/forks/frameworks`, then a 37-agent workflow cataloged each framework's minification-surviving signatures, compared them to the existing pipeline pass (if any), and adversarially verified pass status + signature currency + realistic recon value.

---

## 1. Headline

js-beautify-rs already ships **~13 framework/recon extractor files** — and almost none of them run:

- **Dead (never compiled):** `angular_decompiler.rs`, `angular_metadata.rs`, `angular_route_extractor.rs`, `ngrx_extractor.rs`, `rxjs_annotator.rs`, `mfe_registry_extractor.rs`, `launchdarkly_extractor.rs`, `http_interceptor_extractor.rs`, `webpack_module_resolver.rs`, `webpack_runtime_decoder.rs`, `bundle_diff.rs`, `transmit_sdk_decoder.rs` — **0 `pub mod` declarations** in `mod.rs`/`lib.rs`, zero callers.
- **Mistargeted to one specific app** (not generic):
  - `mfe_registry_extractor.rs` is **single-spa**, not Module Federation, with Schwab-specific name prefixes.
  - `launchdarkly_extractor.rs` requires literal `RelTgl_` flag prefixes and `EMOST-\d{6}` tickets → **0 flags** on any non-Schwab app.
  - `webpack_runtime_decoder.rs` hardcodes one app's JSONP global `lmsJsonp`, mislabels `.r`, and covers ~15 of 30+ runtime suffixes.
- **Broken / stale:**
  - `rxjs_annotator.rs:143-147` bails on **any** line containing `.pipe(` → emits nothing; operator list is RxJS 7.8.1 (clone is **8.x**).
  - `angular_decompiler.rs` decodes only the classic instruction subset — no v17/18 control flow (`ɵɵconditional`/`ɵɵdefer`/`ɵɵdeclareLet`); the clone is Angular **v22**.
- **Architectural prerequisite:** the `extract_*` family is regex/string-in → struct-out, **not** oxc `Traverse` passes. The live recon pipeline is `src/analyzers/mod.rs::run_all` (contract: `analyze(source,&str,FileType) -> Vec<AnalyzerMatch>`). **Adding `pub mod` in `ast_deobfuscate/mod.rs` does nothing — these have no caller.** A dispatcher must be built in `analyzers/run_all` first.

**Frameworks with zero coverage** (need new passes): React/RSC, Vue 3, Svelte 5, Redux/RTK/RTK-Query, Zustand, Vite, Rollup, Sentry, Preact, Solid.

**Verdict spread:** 16 high-value, 2 medium (RxJS, Apollo), 0 low.

---

## 2. Clone manifest (current versions verified)

`~/RustProjects/forks/frameworks/` (sparse, shallow): React **19.3.0** (+ react-compiler), Next.js, Angular **22.1.0-next**, Vue **3.x**, Svelte **5.56.2**, RxJS **8.x**, Redux **5.0.1** + RTK **2.12.0** (+ RTK Query), NgRx, Zustand **5.0.14**, Webpack **5.107.2**, Vite **8.0.16**, Rollup, Module Federation **2.5.1**, Apollo Client **4.2.2**, Sentry, Preact, Solid, LaunchDarkly **3.9.3**.

---

## 3. ARCHITECTURAL PREREQUISITE (gates Tier A and most of Tier C)

Build one dispatcher in the **live** analyzer pipeline so every framework finding becomes a deduped `AnalyzerMatch`:

- Add `src/analyzers/framework_recon.rs` exposing `analyze(source,file_path,file_type) -> Vec<AnalyzerMatch>`; register `pub mod framework_recon;` in `analyzers/mod.rs`; add `out.extend(framework_recon::analyze(...))` to `run_all` behind a default-true `config.framework_recon` flag.
- Inside it, call each `extract_*`/`format_*` and map results into `AnalyzerMatch{ match_kind, match_value, severity, metadata, provenance }`.
- **Exception:** the two AST-native webpack passes (`webpack_runtime_decoder`, `webpack_module_resolver`) mutate/annotate the tree — declare them `pub mod` in `ast_deobfuscate` and call from the post-codegen annotation stage. Everything else lives in `analyzers/`.

---

## 4. Tier A — WIRE (passes that exist; some need update-before-wire)

| Pass | Matches current output? | Action |
|---|---|---|
| `angular_metadata` | YES (ɵcmp/ɵfac anchors still match v22) | **Wire now** (selectors/inputs/outputs); update-after for DI graph |
| `angular_route_extractor` | PARTIAL (webpack `ensureChunk` only) | Wire; update-first preferred for esbuild/Vite `import()` |
| `angular_decompiler` | STALE (classic instr only) | **Update-first** — incomplete on modern apps |
| `ngrx_extractor` | YES for action catalog; effects broken | **Wire now**; update-after for effects |
| `rxjs_annotator` | BROKEN (`.pipe(` line-bail) + stale v7 | **Update-first mandatory** |
| `webpack_module_resolver` | STALE (arrow-only regex) | Wire (AST phase); update package table |
| `webpack_runtime_decoder` | WRONG (`.r` mislabel, `lmsJsonp` hardcode) | **Update-first mandatory** |
| `mfe_registry_extractor` | MISTARGETED — single-spa, not MF | Relabel→`single_spa`, generalize, then wire |
| `launchdarkly_extractor` | currency unverified (Schwab-hardcoded) | Generalize + verify, then wire |
| `http_interceptor_extractor` | currency unverified | Verify signatures, then wire |

**High-leverage detail:**
- **A1 `ngrx_extractor`** — pivot effect detection to the minification-proof key `__@ngrx/effects_create__`; catalog `createAction("[Domain] Event")` (incl. `(0,n.createAction)` namespace form); map slices via `createFeatureSelector`/`createFeature`. Bracketed action-type strings are runtime literals terser can't rename → catalog works immediately.
- **A2 Angular trio** — anchor `X.ɵcmp = ɵɵdefineComponent({selectors:[["app-foo"]],inputs,outputs})` and `Route{path,loadComponent,loadChildren}`. Selectors/inputs/route data survive verbatim and are the bulk of the value even with the incomplete decompiler.
- **A3 `webpack_runtime_decoder` (AST phase)** — replace `lmsJsonp` with generic `(self|window|globalThis)\.webpackChunk[\w$.-]*`; fix `.r`→makeNamespaceObject; extend suffix table to the full `RuntimeGlobals` set. **Add `.u`/`.k` embedded `{chunkId:"name"}` map extraction — the single highest-value webpack recon artifact** (route/vendor/component chunk names + hashes survive verbatim). Identify the mangled require var by its `.d/.r/.o` + `var(int)` fingerprint, not by name.

---

## 5. Tier B — UPDATE (compiled/wired passes that are stale)

- **B1 `graphql_extractor`** (analyzers/, default-on) — emits raw SDL with `metadata=None`; misses renamed `gql` tag (`n.gql`, `i\`…\``), anonymous `gql\`{…}\``, Apollo runtime markers, and precompiled `DocumentNode` objects. Add: leading-token parse → `{operation_type,operation_name,on_type,fragment_names}`; accept `MemberExpression` tags with property `gql/graphql`; match `__APOLLO_CLIENT__`, `persistedQuery`+sha256, `/graphql` default endpoint. **Cheapest high-yield item in the plan.**
- **B2 `nextjs_discovery`** (CLI `--discover-nextjs`) — covers only `buildId` + static paths + pages-router `sortedPages`. Add `__SSG_MANIFEST`, full `__NEXT_DATA__` body, `__RSC_MANIFEST`. (The richest app-router gaps belong in the new C2 analyzer, not bolted onto this fetch helper.)
- **B3 `DisplayNameCollector`** (wired, `mod.rs:515`) — matches only `X.displayName = <StringLiteral>` and **destructively renames** (collides on reused mangled ids, discards `$$typeof` context). Make non-destructive (annotate; rename only when unique); broaden RHS (`Object.assign`, `a || "Name"`, `defineProperty`).
- **B4 `webpack_module_extractor`** (CLI `--extract-modules`) — `try_parse_module` accepts only `function(...)` factories; add arrow `(m,e,r)=>{}` (dominant in webpack-5) and chunk-file `.push([[ids],{id:fn}])`; resolve the require var from the runtime decoder instead of hardcoding `t`.
- **B5 `helpers.rs` sentinel suppression** (~line 97) — `__REDUX_DEVTOOLS_EXTENSION__` is scrubbed as path-noise, discarding the strongest Redux/Zustand devtools signal. Keep it in the secret-noise list but emit a `state_mgmt_devtools` info match when it co-occurs with a store fingerprint.

---

## 6. Tier C — NEW (no pass; ranked by recon value)

- **C1 React 19 + Compiler** (high) — three sub-passes: `react_compiler` (`Symbol.for("react.memo_cache_sentinel")` chain + `_c(N)` prelude → compiled-fn skeleton + slot counts; honest scope: anonymous-with-complexity, **not** a named tree), `react_elements` ($$typeof → `react.transitional.element`/forwardRef/memo/lazy/context tagging), `react_internals` (`__CLIENT_INTERNALS…`, `__COMPILER_RUNTIME`, `{H,A,T,S,G}` shape → version era stamp). Prefilter on `react.memo_cache_sentinel`.
- **C2 Next.js app-router/RSC** (high) — `self.__next_f.push([tag,payload])` flight grammar, `createServerReference("hex",…)` + info-byte decode → **server-action POST endpoints** (high attack surface), `NEXT_PUBLIC_*` secrets, `webpackChunk_N_E`, `.u` chunk map. Feed `__next_f` from `html_script_extractor`.
- **C3 Vue 3** (high) — literal-name signatures survive Vite mangling: `Symbol.for("v-fgt"|"v-txt"|…)`, `__scopeId:"data-v-…"`, `__file`/`__vccOpts`/`__name`, `createStaticVNode`, `resolveComponent("X")`; `createRouter` route arrays; Pinia `defineStore("id",…)`. Recover component registry + scope hashes, routes + lazy chunks, store slices. Demote build-inlined flag globals + PatchFlag matching.
- **C4 Redux + RTK + RTK Query** (high) — `createSlice`/`createAsyncThunk` → action vocab; **RTK Query `fetchBaseQuery({baseUrl})` + `build.query/mutation` endpoints + `tagTypes` = top win (REST surface)**. Detect via `@@redux/INIT`, `redux.js.org/Errors?code=`, `Symbol.for("rtk-slice-createasyncthunk")`. Cross-ref `api_extractor`.
- **C5 Vite** (high) — `vite:preloadError` fingerprint, `__vite__mapDeps=(i,m,…)` chunk+CSS table, `__vitePreload(()=>import(X), __vite__mapDeps([…]))` lazy graph, `import.meta.env` (harvest `VITE_*` secrets), `import.meta.glob` file maps, dev markers `/@vite/client`/`/@fs/`, `.vite/manifest.json`. Low effort for `mapDeps`, huge SPA footprint.
- **C6 Module Federation** (high) — new analyzer (NOT the single-spa `mfe_registry`): `.federation=`, `bundlerRuntimeOptions.remotes.remoteInfos` (**remoteEntry URLs = cross-origin attack-surface expansion, top priority**), exposes map, shared-dep version table (CVE matching), `mf-manifest.json` parser.
- **C7 Sentry** (med-high) — DSN anchored on `ingest.`/`@o<digits>.`/`sentry_key`/`sentry_version=7` (not bare URL regex), `__SENTRY__`, `sentry.javascript.` prefix, integration `.name` literals, `_sentryDebugIds`, `SENTRY_RELEASE/DSN/ENVIRONMENT`, Spotlight `localhost:8969`. Mostly regex. (Debug-ids don't grant source access — only confirm sourcemap upload.)
- **C8 Svelte 5** (high) — string+shape recon (names tree-shaken to 1–2 chars): `from_html('<…>')` hoisted templates (**recover routes/forms/classes/text — top win**), `svelte-<hash>` classes, `store_get(ref,'$name',…)`, `$$css.code`, `window.__svelte.v` version. Mirror `nextjs_discovery`.
- **C9 Rollup** (high, multi-pass) — match by structure not name: interop bodies (`e&&e.__esModule?e.default:e`, `_mergeNamespaces`, `getDefaultExportFromCjs`), finaliser preambles (UMD/IIFE/ESM/`System.register`), vendored-CJS boundaries (CVE). Source `@rollup/plugin-commonjs` separately.
- **C10 Zustand** (high) — api 4-key shape `{setState,getState,getInitialState,subscribe}` + `useSyncExternalStore`; **persist `name` localStorage key (where client secrets live → Medium security finding)**; middleware fingerprints via literal error strings. Don't key on `zustand/persist` (type-only, erased).
- **C11 Preact** / **C12 Solid** (medium, signatures unverified in this corpus — confirm against source first) — Preact `options` hook keys (`_b/_r/_c/__H`), compat shims; Solid `_$template`/`_$insert`/`_$createComponent` dom-expressions output + hoisted `template(\`…\`)`.

---

## 7. DO THIS FIRST — single ordered list (recon value × effort)

1. **Framework-recon dispatcher** in `analyzers/run_all` — prerequisite, unlocks A + most of C. *enabling/low*
2. **Wire `ngrx_extractor`** (action catalog). *high/tiny*
3. **`graphql_extractor` metadata + renamed-tag fix** (B1). *high/low*
4. **Wire Angular `metadata`+`route_extractor`+`decompiler`** (A2) — tests already pass. *high/low*
5. **`react_compiler` + `react_internals`** (C1) — headline 2026 signal. *high/medium*
6. **Vite `vite_runtime` + `import.meta.env`** (C5). *high/low-med*
7. **Next.js app-router RSC** (C2) — flight tree + server-action endpoints. *high/med-high*
8. **Module Federation** (C6) — remoteEntry URLs. *high/medium*
9. **Sentry** (C7) — DSN/endpoint, mostly regex. *med-high/low*
10. **Redux/RTK/RTKQ** (C4) — RTK Query REST surface. *high/medium*
11. **Vue 3 + Router + Pinia** (C3). *high/medium*
12. **`webpack_runtime_decoder` rewrite + `.u` chunk-name map, then wire** (A3). *high/med-high*
13. **Svelte `from_html` template recovery** (C8). *high/medium*
14. **`rxjs_annotator` bug-fix + v8 operator table, then wire**. *medium/low-med*
15. **Rollup interop/format/system** (C9). *high/high*
16. **Zustand** (C10) — persist-key win cheap. *high/medium*
17. **`webpack_module_extractor` arrow+chunk-push** (B4); **`nextjs_discovery` manifests** (B2). *medium/medium*
18. **Relabel `mfe_registry`→single_spa; verify+wire `launchdarkly`,`http_interceptor`**. *medium/low*
19. **`DisplayNameCollector` non-destructive broaden** (B3); **un-suppress devtools sentinel** (B5). *low/low*
20. **Preact, Solid** — verify signatures first. *medium/medium*

**Cross-cutting:** (a) the dispatcher MUST precede everything; (b) `rxjs`/`webpack_runtime_decoder`/`angular_decompiler` are simultaneously WIRE+UPDATE — they emit garbage/nothing without an update first; (c) `mfe_registry` is single-spa, not Module Federation (MF is the new C6); (d) prefer the LIVE `analyzers/` home over `ast_deobfuscate/` for all recon extractors except the two AST-native webpack passes.
