# Mindemon ↔ jsbeautify integration

One repository, two halves: the **Mindemon** Firefox extension (in `frontend/`)
captures everything a web page actually loads in the browser; the
**jsbeautify** Rust crate + daemon (repo root) beautifies, deobfuscates,
analyzes, and reverses source maps on the backend. This document describes how
they connect.

## The two halves

| | Mindemon (capture) | jsbeautify (analysis) |
|---|---|---|
| Location | `frontend/` (this repo) | repo root (`src/`, `Cargo.toml`) |
| Role | Firefox DevTools extension — **capture** | Rust daemon — **analysis** |
| Strength | Sees the real runtime: lazy chunks, workers, WASM, source maps, hydration globals | 20-phase AST deobfuscator, oxc-powered beautifier, 14 security analyzers, source-map reversal, cross-version alignment |

Historically jsbeautify grew a full jxscout-style platform (MITM proxy,
WebSocket server, SQLite project DB, hooks, monitoring) but those modules were
**library-only** — no binary ever ran them. The `--serve` daemon plus the
`pipeline` + `server` modules are the seam that turns them into a live service
and connects them to Mindemon.

## Data flow

```
┌─────────────────────── Firefox ───────────────────────┐
│  Mindemon DevTools panel                              │
│   • Scan Current Target  → collects JS/CSS/maps/...   │
│   • Send to Backend      → POST /ingest/batch         │
└───────────────────────────┬───────────────────────────┘
                            │ HTTP (JSON)
                            ▼
┌──────────── jsbeautify --serve ───────────────────┐
│  HTTP API :3333  ── /ingest, /ingest/batch            │
│        │                                              │
│        ▼                                              │
│   Pipeline:  classify → beautify/deobfuscate →        │
│              analyzers::run_all →                     │
│              sourcemap reverse (sourcesContent) →     │
│              persist (js_files + matches) →           │
│              broadcast                                │
│        │                                              │
│        ├──▶ SQLite  ~/.jsbeautify/projects/<name>/    │
│        └──▶ WebSocket :9222 ──▶ editor clients        │
│                                                       │
│  MITM proxy :8080 ── passive capture → same Pipeline  │
└───────────────────────────────────────────────────────┘
```

## End-to-end usage

1. **Start the backend:**
   ```bash
   cd ~/RustProjects/active/jsbeautify
   cargo build --release
   ./target/release/jsbeautify --serve --project-name myhunt
   ```
   It prints the listening ports and the project workdir.

2. **Load Mindemon** in Firefox (`about:debugging` → Load Temporary Add-on →
   `frontend/manifest.json`), open DevTools → **Mindemon Dump** panel.

3. **Capture & send:** click *Scan Current Target*, then *Send to Backend*
   (the Backend URL field defaults to `http://127.0.0.1:3333`). The panel log
   shows per-asset finding counts.

4. **Inspect results:** beautified/reversed files land under
   `~/.jsbeautify/projects/myhunt/`; findings are queryable at
   `GET http://127.0.0.1:3333/matches`; live events stream over `ws://127.0.0.1:9222`.

## Why HTTP for ingest + WebSocket for results

- **Ingestion is bulk request/response** — a dump of N assets, each wanting an
  individual result. Plain `POST /ingest/batch` fits and needs no connection
  state. This mirrors jxscout's `:3333` design.
- **Results are a live push stream** — as the proxy/monitor discover new files
  the backend must notify already-connected editor clients. That is what the
  WebSocket server (`:9222`) is for.

Using each transport for what it's good at avoids reinventing request/response
over a socket or polling for events over HTTP.

## Contract

See the **HTTP ingestion contract** section of `README.md` for the exact JSON
schema of `/ingest` and `/ingest/batch`. The Mindemon side builds that payload in
`frontend/panel/panel.js` → `buildBackendPayload()` / `sendToBackend()`.
