"use strict";
const test = require("node:test");
const assert = require("node:assert");

// source-map.js is an IIFE that attaches to globalThis.MindemonSourceMaps.
require("../lib/source-map.js");
const SM = globalThis.MindemonSourceMaps;

test("module exposes the expected API", () => {
  assert.ok(SM, "MindemonSourceMaps should be defined");
  for (const fn of ["extractSourceMappingURL", "extractDebugId", "guessMapURL", "resolveForText"]) {
    assert.strictEqual(typeof SM[fn], "function", `${fn} should be a function`);
  }
});

test("extractSourceMappingURL: // and /* */ comment forms", () => {
  assert.strictEqual(SM.extractSourceMappingURL("a=1;\n//# sourceMappingURL=app.js.map"), "app.js.map");
  assert.strictEqual(SM.extractSourceMappingURL("a=1;\n//@ sourceMappingURL=app.js.map"), "app.js.map");
  assert.strictEqual(SM.extractSourceMappingURL("a=1;\n/*# sourceMappingURL=app.css.map */"), "app.css.map");
});

test("extractSourceMappingURL: last one wins, null when absent", () => {
  assert.strictEqual(
    SM.extractSourceMappingURL("//# sourceMappingURL=first.map\n//# sourceMappingURL=second.map"),
    "second.map"
  );
  assert.strictEqual(SM.extractSourceMappingURL("no map here"), null);
});

test("extractDebugId: // and /* */ forms", () => {
  assert.strictEqual(
    SM.extractDebugId("code;\n//# debugId=1234abcd-5678-90ef-aaaa-bbbbccccdddd"),
    "1234abcd-5678-90ef-aaaa-bbbbccccdddd"
  );
  assert.strictEqual(
    SM.extractDebugId("code;\n/*# debugId=deadbeefdeadbeef */"),
    "deadbeefdeadbeef"
  );
  assert.strictEqual(SM.extractDebugId("no debug id"), null);
});

test("guessMapURL: appends .map, strips query/fragment", () => {
  assert.strictEqual(SM.guessMapURL("https://x/app.123.js"), "https://x/app.123.js.map");
  assert.strictEqual(SM.guessMapURL("https://x/app.js?v=2"), "https://x/app.js.map");
  assert.strictEqual(SM.guessMapURL("https://x/app.js#z"), "https://x/app.js.map");
  assert.strictEqual(SM.guessMapURL(""), null);
});

test("resolveForText: reverses an inline data: source map (no network)", async () => {
  const map = {
    version: 3,
    sources: ["src/login.ts", "src/api.ts"],
    sourcesContent: [
      "export function login() { return fetch('/api/auth'); }",
      "export const API = '/api/v2';",
    ],
    names: [],
    mappings: "",
  };
  const dataUrl = "data:application/json;base64," + Buffer.from(JSON.stringify(map)).toString("base64");
  const content = `console.log(1);\n//# sourceMappingURL=${dataUrl}`;

  const result = await SM.resolveForText(
    { url: "https://x/app.js", content },
    { pageURL: "https://x/", allowExternalOrigins: true, fetchMissingSources: false }
  );

  assert.strictEqual(result.originals.length, 2);
  assert.strictEqual(result.originals[0].source, "src/login.ts");
  assert.ok(result.originals[0].content.includes("login"));
  assert.strictEqual(result.originals[1].source, "src/api.ts");
});

test("resolveForText: returns debugId even when no map is resolvable", async () => {
  const content = "console.log(1);\n//# debugId=abcdef12-3456-7890-abcd-ef1234567890";
  const result = await SM.resolveForText(
    { url: "https://x/app.js", content },
    { pageURL: "https://x/", allowExternalOrigins: false, fetchMissingSources: false }
  );
  // No explicit map URL and the guessed .map is out of scope / unfetchable, but
  // the debugId must still be surfaced.
  assert.strictEqual(result.debugId, "abcdef12-3456-7890-abcd-ef1234567890");
  assert.strictEqual(result.mapSource, "debugId-guess");
});
