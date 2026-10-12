Turbopack output used by `tests/integration/turbopack.rs`. All but one file are
generated chunks copied unchanged from next.js v16.5.0-canary.8,
`turbopack/crates/turbopack-tests/tests/snapshot/`:

- `dev_browser_chunk.js`: `basic/async_chunk/output/1do3_…_1lkohl-aes1w9._.js`
  (browser, development, string module ids, all-strict wrapper)
- `dev_node_chunk.js`: `debug-ids/node/output/1do3_…_1gqemi9q9l5m7.js`
  (Node.js, development, debug-id prelude)
- `node_strict_minified.js`: `mangle-exports/single-export/output/0_9x_…_1mf78g4ogdhgv._.js`
  (Node.js, minified, all-strict wrapper, mangled export key `f`)
- `node_entry.js`: `runtime/default_build_runtime/output/index.entry.js`
- `worker_entrypoint.js`: `workers/basic/output/0uxq_…_output_0uy0mninb8pht._.js`

`dev_chunk_list.js` follows the template in
`turbopack/crates/turbopack-browser/src/ecmascript/list/content.rs` (chunk lists
are served by the dev server and have no snapshot).
