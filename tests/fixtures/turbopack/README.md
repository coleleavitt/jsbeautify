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

The `next16_*.js` files are copied unchanged from a real Next.js 16.4.0 app
built with Turbopack (`create-next-app`, see `~/SiteResearch/turbopack/REAL_BUILD.md`):

- `next16_nested_server_chunk.js`: `.next/server/chunks/ssr/[root-of-the-server]__0py1t7q9ntzgc._.js`
  (Node.js, the leading nested factory list Next.js 16.4 emits as a plain array)
- `next16_edge_entry_chunk.js`: `.next/server/edge/chunks/[root-of-the-server]__00cre0x0mv0lu._.js`
  (edge, `self._ENTRIES["middleware_app/api/edge/route"]` wrapper)
- `next16_require_context_chunk.js`: `.next/server/chunks/ssr/_0y55_8mx16aml._.js`
  (`require.context` map module 52812)
- `next16_merged_modules_chunk.js`: `.next-nominify/server/chunks/ssr/[root-of-the-server]__026w794mlib5p._.js`
  (`experimental.turbopackMinify: false`: `// MERGED MODULE:` markers, `S([…])` re-exports)
- `next16_dev_refresh_chunk.js`: `.next/dev/static/chunks/src_app_heavy_tsx_0a3zll2ka71z6._.js`
  (`next dev`, React Refresh `__turbopack_context__.k.register`)
