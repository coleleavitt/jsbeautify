# In-browser vs out-of-browser: where mindemon-runtime-dumper belongs

## The actual question
"Should I just make a Burp/Caido plugin? What belongs *inside* the browser vs *outside*?"

Answer: **it's not either/or — the tool is already correctly split, and a pure proxy plugin
would lose the exact things that made our Orion recon work.** Here's the principled boundary.

## What MUST be inside the browser (can't be done from a proxy)
These are the capabilities that won the Orion job — none are available to Burp/Caido:
1. **The webpack `__webpack_require__.u` chunk-map → lazy-chunk enumeration.** A proxy only
   sees chunks the user *navigated to*. Inside the page you read the runtime chunk map and
   enumerate **unloaded** chunks (the whole point of the recent bug fix). The Communities TAMP
   app's 6 MB real bundle was a lazy chunk never loaded at the dashboard — only in-page map
   parsing finds it.
2. **Real credentialed fetch in the page's origin context.** Our authed captures (Billing,
   Copilot, the Eclipse `Session`/`eclipse_access_token` exchange) worked because the extension
   issues `fetch()` *as the page* — same cookies, same `Authorization` the SPA already holds, same
   CORS/SameSite. A proxy can replay captured requests but can't mint new authenticated calls in
   the live session without re-implementing the whole auth dance.
3. **Runtime/JS-only state** a proxy never sees: `document.scripts[]` inline bodies, hydration
   globals (`__NEXT_DATA__`, `__remixContext`), import maps, Blob/worker bodies created in-memory,
   Service-Worker precache manifests, `import.meta.url` assets, SPA router state, the resolved
   sourcemap `sourcesContent`. (This is most of the README's capability list.)
4. **DevTools network capture of *actual response bodies*** via
   `devtools.network.onRequestFinished + getContent()` — including bodies served from cache /
   HTTP2 push that a MITM proxy may not re-see.

## What belongs outside the browser (already is, via the native host)
The heavy, persistent, untrusted-input work stays in Rust:
1. **Deobfuscation + AST rename** (oxc) — too heavy for a content script; needs the Rust
   `ast_deobfuscate` engine.
2. **Sourcemap resolution, chunk_detector token parsing, webpack module extraction.**
3. **Persistence, the recon graph, endpoint/secret analyzers, the vuln DB, reporting.**
4. **Anything that should survive the tab closing / run across many dumps.**

The bridge between them is **native messaging** (`frontend/native/mindemon_host.py` ↔
`nativeMessaging` perm) — the extension captures, the host/Rust analyzes. That's the right cut.

## So: Burp/Caido plugin? — No (as the primary), maybe as an *adjunct*
- **Burp/Caido = a MITM proxy.** Its superpower is request **interception/modification/replay**
  and coverage of *non-browser* clients. Its blind spot is exactly (1)-(4) above — it has **no
  JS runtime**, so it cannot enumerate lazy chunks, read the chunk map, see SPA/runtime state, or
  issue in-session authenticated fetches. For a *source/asset/runtime dumper*, that's
  disqualifying as the core.
- **Where a Caido/Burp plugin DOES help** (complementary): passive capture of API traffic for
  fuzzing/replay, scope management, and feeding captured endpoints back. If you want
  request-tampering + repeater, add a thin Caido plugin that **consumes** the Rust core's endpoint
  catalog — don't move the dumper into it.

## The forked-browser option (camoufox / BotBrowser / cloakbrowser)
Now relevant because of *how Orion served things* (Cloudflare TLS-fingerprinting blocked Node
`fetch`; only curl/real-browser TLS passed — see the pplx finding). A forked anti-detect browser
is the answer to **"the target blocks automated/headless clients,"** NOT a replacement for the
extension:
- **camoufox** (`~/CLionProjects/forks/camoufox`, 1.5 GB, 36 Firefox patches) — patched **Firefox**;
  C++/JS-level fingerprint spoofing (navigator, canvas, WebGL, fonts, screen) + a Playwright-style
  Python lib. Best fit because **our extension is already Firefox DevTools** — the dumper extension
  could load *inside* camoufox to get stealth + runtime capture in one.
- **BotBrowser** (107 MB, Chromium patches) — same idea for Chromium; unified fingerprint defense.
- **cloakbrowser** (14 MB) — "stealth Chromium, drop-in Playwright replacement, source-level
  fingerprint patches" — a *library* wrapper, lightest, good for automated/headless driving.

These solve the **access** problem (get past Cloudflare/bot-detection to *reach* the assets),
which is orthogonal to the **extraction** problem (the extension) and the **analysis** problem
(the Rust core).

## Recommended architecture (three layers, each doing what only it can)
```
┌─ ACCESS ─────────────┐   ┌─ EXTRACTION (in-browser) ─┐   ┌─ ANALYSIS (out) ──────────┐
│ camoufox (Firefox)   │ → │ mindemon DevTools ext     │ → │ Rust core (native host)   │
│ stealth TLS/FP, real │   │ chunk-map enumerate,      │   │ deob, sourcemap resolve,  │
│ session, beats bot   │   │ credentialed fetch,       │   │ chunk_detector, recon,    │
│ detection            │   │ runtime/DOM/SW/maps       │   │ vuln db, reports          │
└──────────────────────┘   └───────────────────────────┘   └───────────────────────────┘
        (+ optional Caido plugin consuming the endpoint catalog for replay/fuzz)
```
- Keep the **extension** as the extraction layer (it's correct and unique).
- Run it **inside camoufox** when targets gate on fingerprint/TLS (Firefox-native = clean fit).
- Keep the **Rust native host** as the analysis layer.
- Add a **Caido plugin only as an adjunct** for request replay/fuzzing of the discovered endpoints
  — not as the dumper.

## One-line answer
Inside the browser belongs everything that needs the **JS runtime + the page's auth/origin**
(chunk enumeration, credentialed fetch, runtime state) → the extension. Outside belongs **heavy
analysis + persistence** → the Rust host. Burp/Caido can't do the inside part, so it's an adjunct,
not the home. The forked browsers (camoufox first, it's Firefox like the extension) solve a
different axis — *access/anti-detection* — and compose with, rather than replace, the extension.
