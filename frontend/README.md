# Mindemon Runtime Dumper

Firefox DevTools extension for dumping JavaScript, CSS, source maps, and static
assets from the inspected page.

It collects:

- Script, stylesheet, font, image, media, WASM, JSON, and source-map resources
  discovered from the page runtime
- Lazy chunk references mined recursively from fetched Vite/webpack/Rolldown-style
  bundles and CSS
- Vite/Rolldown semantic dependency hints, including `__vite__mapDeps`,
  modulepreload dependency arrays, manifest `file`/`css`/`assets`/`imports`/
  `dynamicImports`, dynamic/static imports, and `new URL(..., import.meta.url)`
- Three.js loader side assets such as DRACO and KTX2 decoder/transcoder files
- `<script type="importmap">` import/scope targets (same-origin only)
- Web Workers, SharedWorkers, and Service Worker scripts (via runtime hooks), plus
  their `importScripts()` dependencies through the recursive same-origin crawl
- Workbox/service-worker precache manifest asset URLs as first-class crawl
  candidates
- Blob-backed worker bodies and their import references when the runtime hook is
  installed before `Blob`/`URL.createObjectURL` construction
- WebAssembly modules loaded via streaming APIs or `fetch(...).arrayBuffer()`
  followed by `WebAssembly.instantiate`/`compile`
- Inline scripts and styles from the page
- Inline data blobs: `application/json`, `ld+json`, and `speculationrules` scripts
- Hydration state globals (`__OMELETTE_ME__`, `__NEXT_DATA__`, `__remixContext`, …)
  snapshotted into `inline/data/hydration-globals.json`
- Runtime load events captured after installing the hook
- Source maps referenced by generated JS/CSS, including modern builds that ship a
  `//# debugId=` instead of a `//# sourceMappingURL=` comment (the `.map` sibling
  is guessed and used when it resolves)
- Original source files from `sourcesContent` or fetchable source-map URLs
- Pretty-printed fallbacks for generated minified files when no source-map original is available
- Assets from **same-origin iframes** (walked recursively, tagged with a frame
  path) and **cross-origin iframe** document URLs (queued as HTML to fetch)
- The **actual response bodies the page received**, captured passively via
  `devtools.network.onRequestFinished` + `getContent()` (see *Network capture*)

All recursive/deep discovery is scoped to the page origin and origins the page
actually loaded. With `First-party family` enabled (on by default), recursive
mining also follows the inspected page's registrable domain plus any explicit
comma-separated domains entered in the panel. `Deep crawl external origins`
removes the origin restriction entirely. It dumps assets the page genuinely loads
or declares; it never probes API/RPC endpoints.

With `Use session cookies` enabled (on by default), in-scope page-origin,
loaded-origin, and first-party-family fetches use `credentials: "include"` so
Firefox can attach the current browser session cookies. Arbitrary external deep
crawl targets still use `credentials: "omit"` unless they are also in scope. The
extension cannot manually inject raw `Cookie` headers; cookie attachment remains
controlled by Firefox, host permissions, and the site's cookie attributes.

For Cloudflare Access protected asset hosts, enter a service token in `CF client
id` and `CF client secret`. The extension sends `CF-Access-Client-Id` and
`CF-Access-Client-Secret` only to hosts listed in `CF domains`; if that field is
blank, headers are limited to the same in-scope page, loaded-origin, and
first-party-family origins used for credentialed fetches. Token values are kept
in panel memory for the scan and redacted from dump metadata; only header names
are recorded.

Crawling is still bounded by the same asset-extension filter, the configurable
per-scan candidate cap, and recursion depth. The first-party family uses a
registrable-domain match, so look-alike suffix spoofs (e.g.
`example.com.evil.io`) are rejected.

## Load in Firefox

1. Open `about:debugging#/runtime/this-firefox`.
2. Click `Load Temporary Add-on`.
3. Select `manifest.json` from this directory.
4. Open DevTools on a page and select the `Mindemon Dump` panel.

## Workflow

1. Click `Install Runtime Hook` before interacting with the page if you want dynamic script/style load events.
2. Click `Scan Current Target`.
3. Click `Dump to /tmp`, `Download ZIP`, or `Send to Backend`.

## Network capture & Reload

The panel listens to `devtools.network.onRequestFinished` from the moment it
opens and buffers the **actual** response bodies the page received (via
`request.getContent()`), keyed by URL. With `Prefer network capture` enabled (on
by default), scanning serves those captured bytes instead of re-fetching — so
you see exactly what the page loaded, including responses that a re-fetch would
get wrong (cache-busted, A/B-tested, nonce'd) or that would fail entirely
(one-time, auth-gated, or CSP-blocked URLs). The `network captured` stat shows
how many responses are buffered. The buffer is memory-bounded (per-entry and
total caps); when a cap is hit the scan falls back to fetching.

Anything the page loaded *before* the panel opened isn't in the capture buffer.
Click `Reload & Capture` to reload the page with the runtime hook injected at
document-start (via `inspectedWindow.reload({ injectedScript })`); the network
listener records every response from the first byte. Wait for the page to settle,
then `Scan Current Target`.

The page snapshot is sent back over the DevTools eval bridge, which JSON-
serializes the whole payload. To avoid truncation/OOM on large apps, inline
script/style/data text and inline-stylesheet rules are capped (with per-field +
total budgets) and `meta/page-snapshot.json` records a `truncation` map; the full
bytes are always recovered through the fetch / network-capture path. Cross-origin
and `href`-backed stylesheets are not read via `cssRules` (that throws
cross-origin and just duplicates bytes) — their `.css` is fetched/captured
instead.

## Send to Backend (js-beautify-rs)

`Send to Backend` POSTs the captured assets to a running
[js-beautify-rs](https://github.com/coleleavitt/js-beautify-rs) daemon, which
beautifies, deobfuscates, runs security analyzers, and reverses source maps,
persisting findings to a project database and streaming results over WebSocket.

1. Start the backend: `jsbeautify --serve --project-name <name>` (defaults to
   `http://127.0.0.1:3333`).
2. Set the panel's `Backend URL` field (default `http://127.0.0.1:3333`).
3. Scan, then click `Send to Backend`. The log shows per-asset finding counts.

The panel sends a `POST /ingest/batch` request:

```json
{
  "target": "https://site/",
  "files": [
    {
      "url": "https://site/app.js",
      "content_type": "application/javascript",
      "content": "…source or base64…",
      "encoding": "utf8",
      "source": "fs_ingest",
      "source_map_content": "{…optional inline source map…}"
    }
  ]
}
```

Text assets are sent as UTF-8; binary assets (fonts, images, WASM) are
base64-encoded with `"encoding": "base64"`. Source maps captured by Mindemon are
attached inline on their generated file as `source_map_content` so the backend
recovers the original `sourcesContent` files and re-analyzes them. See
`INTEGRATION.md` in the js-beautify-rs repo for the full contract.

The output layout is:

- `generated/` for runtime-discovered and bundle-referenced resources
- `original/` for source-map originals
- `source-map/` for fetched source maps
- `inline/` for inline scripts/styles
- `pretty/` for readable fallback output
- `meta/` for page snapshot, runtime events, resource metadata, fetch/source-map
  errors, framework detection, service-worker precache inventory, and capability
  dependency classification

## Dumping to `/tmp`

Firefox extensions cannot write arbitrary local directories by themselves. To dump an unpacked folder under `/tmp/mindemon-dumps/`, install the native messaging host:

```bash
npm run install:native
```

Then reload the temporary extension in `about:debugging` and use `Dump to /tmp`. The extension streams files to the host in chunks, so this path is better than ZIP for larger apps.

To remove the native host:

```bash
npm run uninstall:native
```

`Download ZIP` does not need the native host and remains the fallback path.

## Limits

This is an unprivileged WebExtension. It cannot directly read Firefox's internal DevTools Redux store or private RDP packets. It uses the public DevTools extension APIs plus page-context runtime instrumentation.

Source-map originals are only clean when the site ships `sourcesContent` or the
original source URLs are fetchable. Source-map auth pages and HTML soft-errors
are recorded in `meta/errors.json` and on the generated resource's manifest
entry instead of being reported only as JSON parse failures. Otherwise the
extension keeps the generated file and can add a pretty fallback.

Worker/WASM capture relies on the runtime hook being installed *before* those
resources are created; anything constructed earlier is missed. `importScripts()`
and worker dependencies are mined by static reference extraction from the fetched
worker text, so dynamically-constructed paths are not always followed.
Synchronously inspectable Blob worker bodies are saved under `inline/blob/` and
their import references are queued, but `data:` workers and blobs built from
async `Blob` parts still have no same-origin URL to re-fetch.
