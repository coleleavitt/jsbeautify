# Bun bundler runtime — extracted reference

These three files were extracted from the Bun (1.4+) native binary by reading
adjacent C-string literals in the IDA decompilation of the bundler's source-map
emitter (`sub_30B7010`, `sub_33C2870`, `sub_33C2990`).

They are **the ground truth** for what Bun's bundler emits at the top of every
single-file executable.

| File | Source | Notes |
|---|---|---|
| `preamble_canonical.js` | `sub_30B7010` | Hand-formatted commented form (with `export var __commonJS` etc). |
| `preamble_emitted_minified.js` | `sub_33C2870` | Exact minified form Bun emits — useful as a fingerprint. |
| `preamble_full_with_modules.js` | `sub_33C2990` | Same minified preamble + several real CJS module bodies, showing how `__commonJS` / `__toESM` get used. |

## Canonical helper names → mangled forms in cli.2.1.132.js

| Canonical | Bundle (mangled) | Body shape |
|---|---|---|
| `__create` | `Sd4` | `Object.create` |
| `__getProtoOf` | `Rd4` | destructured from `Object` |
| `__defProp` | `fjH` | destructured from `Object` |
| `__getOwnPropNames` | `UP8` | destructured from `Object` |
| `__getOwnPropDesc` | `Cd4` | destructured from `Object` |
| `__hasOwnProp` | `XG$` | `Object.prototype.hasOwnProperty` |
| `__accessProp` | `JG$` | `function(key){return this[key]}` |
| `__copyProps` | `g6` | for-of over getOwnPropNames |
| `__toESM` | `m` | `(mod, isNodeMode, target) => …Sd4(Rd4(mod))…` |
| `__toCommonJS` | `n6` | WeakMap-cached `__esModule` wrapper |
| `__commonJS` | `n` | `(H,$) => () => ($\|\|H(($={exports:{}}).exports,$), $.exports)` |
| `__name` | `md4` | `Object.defineProperty(t,"name",{value, …})` |
| `__exportSetter` | `md4` (close) | `function(name, newValue) { this[name] = …bind(null, newValue) }` |
| `__export` | `j$` | `for (var q in $) fjH(H,q,{get:$[q],…})` |
| `__esm` | `V` | `(H,$) => () => (H && ($=H(H=0)), $)` ← **the lazy thunk** |

## Why this matters for js-beautify-rs

`EsbuildHelperCollector` (in `src/ast_deobfuscate/esbuild/helper/`) already
detects all of these by AST shape. **It returns 0 helpers on the Bun bundle**
because Bun wraps the entire program in a single CJS-style IIFE:

```js
(function(exports, require, module, __filename, __dirname) {
    // ... entire program here, including `var V = (H,$) => () => (...)`
})
```

The collector only walks top-level `program.body`, never descending into the
IIFE. Fix: add an unwrap pass that also scans the body of a single
top-level wrapper IIFE.
