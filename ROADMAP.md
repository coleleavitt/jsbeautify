# js-beautify-rs → autonomous JS recon brain — ROADMAP

A prioritized gap analysis for turning this from a beautifier/deobfuscator into a
self-driving reconnaissance + vulnerability-discovery engine. Each item is
grounded in the research corpus under `~/VulnerabilityResearch/_corpus/` (37
papers, full text in `txt/`) and mapped to concrete Rust + oxc work.

## Where we are today (shipped)

- **Beautify + deobfuscate**: control-flow unflattening, string-array decoding,
  esbuild/webpack/rollup unbundling, Akamai/Angular decompilers (`src/ast_deobfuscate/`, 88 files).
- **Analyzers**: secrets (regex bank), sinks (`innerHTML`, `postMessage`,
  `location` assignment), API paths, hostnames, GraphQL, npm packages, framework
  recon (`src/analyzers/`, 32 files).
- **Source-map reversal** (`src/cross_version/`), chunk detection/splitting.
- **NEW — whole-domain mining**: `domain_recon` (crt.sh CT-log subdomain
  enumeration, `--crtsh`) and `recon_engine` (recursive crt.sh → fetch
  JS/HTML/source-maps → deobfuscate → analyze → harvest new hosts/endpoints/
  secrets → loop, `--recon`).
- **NEW — taint-lite source→sink analysis** (`analyzers/taint.rs`): intra-procedural
  DOM-XSS / open-redirect detection (`dom_xss`, `open_redirect` findings). First
  step of the P0 taint engine below. ✅ shipped.
- **NEW — entropy secret scoring** (`analyzers/entropy_secret.rs`): Shannon-entropy
  + charset heuristics flag high-entropy credential-shaped strings and ignore the
  low-entropy URL/prose noise that drowns regex scanners. ✅ shipped.
- **NEW — unified LLM client** (`llm.rs`): one OpenAI-compatible client honoring
  standard `OPENAI_API_KEY`/`OPENAI_BASE_URL`/`OPENAI_MODEL` plus `--llm-url/
  --llm-model/--llm-key` flags; powers the `--rename-llm` tier. ✅ shipped.
- **NEW — CWE + OWASP taxonomy tagging** (`analyzers/taxonomy.rs`): every finding
  gets `{cwe, owasp}` metadata via a local mapping. ✅ shipped.
- **NEW — IDOR candidate detection** (`analyzers/idor.rs`): flags
  client-supplied object-reference endpoints (`idor_candidate`, CWE-639). ✅ shipped.
- **NEW — known-CVE catalog** (`vuln_db.rs`, `--vuln-scan`): maps detected
  library versions → OSV CVEs/GHSAs + flags CISA-KEV exploited-in-the-wild
  (`known_cve` findings). ✅ shipped.

The capture extension only ever mined assets the inspected page referenced
(`frontend/panel/panel.js` `createDeepCrawlScope` seeds `allowedOrigins` from the
page itself), so a `/authplatform/` capture saw one origin. `recon_engine` closes
that gap from the backend.

---

## P0 — Static taint analysis (the single biggest leap) — STARTED ✅

A first **intra-procedural, flow-insensitive** pass ships in `analyzers/taint.rs`
(sources: `location.*`, `document.URL/cookie/referrer`, `window.name`, message
`.data`; sinks: `innerHTML`/`outerHTML`/`document.write`/`eval`/`location` assign
+ `assign`/`replace`). It already catches the BlackRock `idpRedirect` open-redirect
shape. Remaining work to reach the full bar below:

Today the rest of the analyzers are **pattern matchers**: they flag
`el.innerHTML = x` but can't tell if `x` is attacker-controlled. Real client-side
bug finding needs **inter-procedural source → sink dataflow** — the next steps:

- **IFDS taint with access paths** (`taint-ifds-accesspaths-2021.pdf`) — the
  practical algorithm: model taint as an IFDS graph-reachability problem over
  *access paths* (`obj.a.b`), giving fast + precise interprocedural results.
  This is the recommended core engine.
- **P/Taint: unified points-to + taint** (`ptaint-points-to-taint-2017.pdf`) —
  compute taint *as* a points-to analysis; one Datalog-style fixpoint yields both.
  Good fit for a from-scratch Rust solver.
- **InspectJS / taint-spec inference** (`taint-inspectjs-2021.pdf`,
  `taint-djangochecker-2020.pdf`) — bootstrap source/sink/sanitizer specs instead
  of hand-listing them.

**Rust+oxc plan**: extend `analyzers/taint.rs` from name-based intra-procedural
tracking to a CFG + def-use IFDS solver over `oxc_semantic` (resolved `SymbolId`s,
access paths), add inter-procedural propagation via a call graph, and model
sanitizers. Output: full source→sink traces, not just the single-function hits we
report today.

## P0 — DOM XSS + open-redirect detection (first taint client)

The BlackRock `user-auth.js` `idpRedirect()` (`window.location.href =
url.toString()` from a webfinger response) is exactly the open-redirect shape.

- **Riding out DOMsday** (`riding-out-domsday-domxss-2018.pdf`) and
  **persistent client-side XSS** (`persistent-clientside-xss-2019.pdf`) — taxonomy
  of real-world DOM-XSS source/sink pairs and storage-based persistence to encode
  as taint rules.

**Plan**: ship a curated source/sink/sanitizer ruleset on top of the P0 engine;
emit a `dom_xss` / `open_redirect` finding kind with the full trace.

## P1 — Prototype-pollution detection

- **Silent Spring** (`protopollution-silent-spring-2022.pdf`) — finds end-to-end
  prototype-pollution + gadget chains in Node; the gadget-detection methodology
  transfers to client JS.
- **GHunter** (`protopollution-ghunter-2024.pdf`) — universal pp gadgets in JS
  runtimes; dynamic but the gadget catalog is reusable as static signatures.
- **Dynamic taint for pp** (`protopollution-dynamic-taint-2023.pdf`),
  **object-lookup analysis** (`object-lookup-prototype-pollution-2021.pdf`).

**Plan**: detect the canonical recursive-merge / `obj[a][b]=c` pattern via the
taint engine, flag `__proto__`/`constructor`/`prototype` key flows.

## P1 — LLM-assisted variable renaming + triage

- **CASCADE** (`deobf-cascade-google-2025.pdf`) — Google's production hybrid: a
  Gemini model + a JS IR (JSIR) for deobfuscation transforms. The IR-grounded
  approach (LLM proposes, static pass verifies) is the template.
- **Usage-context renaming** (`rename-varnames-usagecontext-2019.pdf`),
  **JsDeObsBench** (`deobf-jsdeobsbench-2025.pdf`, eval harness),
  **From Obfuscated to Obvious** (`deobf-obfuscated-to-obvious-2025.pdf`),
  **LLM4Decompile** (`decompile-llm4decompile-2024.pdf`).

**Plan**: optional `--rename-llm` tier already stubbed — feed each function's
deobfuscated body + usage context to an LLM, constrain to rename-only edits, then
re-parse to verify semantics are preserved (CASCADE's verify loop).

## P1 — Source-map harvesting at scale

`recon_engine` already follows `//# sourceMappingURL`. Extend to: probe common
map locations (`.js.map`, `/static/*.map`), parse `sourcesContent` for original
source, and reconstruct directory trees (we did this manually for the okta
widget's 5 MB map). Recovered originals feed the analyzers with real identifiers.

## P1 — Agentic LLM analysis tier (next)

Static detectors cover the bug classes with a structural signature (XSS, open
redirect, IDOR-candidates, known CVEs). The classes that *don't* have a signature
— business-logic flaws, auth bypasses, subtle access-control gaps — need
reasoning, not pattern matching. Plan: an **agent loop** (`agent/` crate built on
the RMCP rust-sdk `#[tool]` pattern in `~/RustProjects/forks/rust-sdk`) where the
LLM is given tools (`get_findings`, `read_source_window`, `query_osv`,
`grep_symbol`, `classify_cwe`) and iteratively investigates a target: triages
static findings, proposes additional vuln classes, and writes a report. The
catalog stays embedded (CWE/OWASP/OSV are authoritative + finite); the LLM does
the open-ended reasoning over it. `llm.rs` already provides the chat client.

## P2 — Broader recon sources (beyond crt.sh)

crt.sh is one CT front-end and flaky (we added retries). Add: passive DNS, the
CT logs directly (certstream), `wayback`/CommonCrawl URL mining, and
permutation/alteration scanning.

- **Certifiably Vulnerable / CT-log recon** (`recon-ct-logs-vulnerable-2023.pdf`,
  `cloud-watching-attacks-2023.pdf`) — using CT logs to find exposed apps;
  validates the crt.sh approach and suggests live-streaming CT.

## P2 — Endpoint/value resolution

Many endpoints are built dynamically (`` `/api/${v}/x` ``, string concat). Add
**interprocedural constant/string folding** (we already have `algebraic_simplify`
+ `array_unpack` passes) to resolve template literals and concatenations into
concrete paths before extraction — recovering endpoints regex misses.

## P3 — Secret detection beyond regex

Add Shannon-entropy scoring and optional live validity checks (is this AWS key
active?) to cut the false-positive noise (e.g. the 599 JSON-LD `basic_auth_url`
hits and Okta SDK property-name "secrets" we saw).

---

## Suggested execution order

1. **P0 taint engine** (IFDS/access-paths over oxc_semantic) — unlocks DOM-XSS,
   open-redirect, prototype-pollution, and dynamic-endpoint resolution.
2. **DOM-XSS + open-redirect ruleset** on top of it (validate against the
   BlackRock `idpRedirect` lead).
3. **Source-map harvesting + dynamic endpoint folding** (cheap, high recall).
4. **LLM rename/triage tier** (CASCADE-style verify loop).
5. **Recon source expansion** (passive DNS, wayback, certstream).
6. **Entropy/validity secret scoring**.

## Corpus

Full text of all 37 papers: `~/VulnerabilityResearch/_corpus/txt/`. PDFs in
`pdfs/`. Re-fetch with `~/VulnerabilityResearch/_corpus/download.sh`.
