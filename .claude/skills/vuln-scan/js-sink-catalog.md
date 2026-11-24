# JS sink catalog — auditing deobfuscated output (vuln-scan phase 2)

Reference for the `codegraph-vuln-analyst` when the target is **deobfuscated
JavaScript** (the malicious/obfuscated scripts this tool reverses — e.g. Akamai
bot-detection, fingerprinting, skimmers) rather than the Rust codebase. The goal
flips: you're not finding bugs in *our* code, you're characterizing what the
*analyzed* JS **does** — its capabilities, exfiltration, and dynamic-execution
surface. Source→sink reachability over the deobfuscated AST still applies; run
codegraph against the reversed output (`--reverse-sourcemap` / reconstructed tree)
once it's indexed.

## Sources (attacker- or environment-controlled input)
`location.{href,search,hash}`, URL query/fragment params, `document.{cookie,referrer}`,
`postMessage` event `.data`, `window.name`, `localStorage`/`sessionStorage`,
`XMLHttpRequest`/`fetch` responses, `document.URL`, form fields, WebSocket frames.

## Sinks by capability

| Capability | Sinks | Why it matters |
|---|---|---|
| **Dynamic code exec** (CWE-95) | `eval`, `new Function`, `setTimeout`/`setInterval`(string arg), `document.write`, `import()` with dynamic specifier | The payload runs attacker-built code; the #1 thing obfuscation hides |
| **DOM XSS** (CWE-79) | `innerHTML`/`outerHTML`/`insertAdjacentHTML`, `document.write`, `el.src`/`el.href` assignment, `setAttribute('on*'…)`, jQuery `.html()` | Source→sink = injection |
| **Exfiltration / beacon** | `fetch`/`XMLHttpRequest.send`, `navigator.sendBeacon`, `new Image().src=`, `WebSocket`, `EventSource`, dynamic `<script src>` | Where collected fingerprint/credential data leaves; trace what flows in |
| **Fingerprinting** | `navigator.*`, `screen.*`, `canvas.toDataURL`, `WebGLRenderingContext`, `AudioContext`, `Date.getTimezoneOffset`, font/plugin enumeration | Bot/anti-fraud signal collection — inventory what's read |
| **Prototype pollution** (CWE-1321) | `Object.prototype`/`__proto__`/`constructor.prototype` writes, recursive merge/extend over attacker JSON | Gadget for downstream exec |
| **Open redirect / nav** | `location`/`location.href`/`.assign`/`.replace`/`window.open` from a source | Redirect to attacker origin |
| **Persistence** | `document.cookie`=, `localStorage.setItem`, `indexedDB`, service-worker `register` | Survives reload; track what's stored |

## Method (per audit)
1. Index the deobfuscated output with codegraph; `codegraph_arch` to map it.
2. `codegraph_xref` each sink above to find call sites; `codegraph_paths` from a
   **source** to each reached sink — the chain is the capability proof.
3. Classify with the real taxonomy (CWE-95/79/1321) via web lookup; for *malware*
   characterization, describe behavior + IOCs (exfil URLs, collected fields), not CVSS.
4. Report: capability, the source→sink chain, the data that flows, and the IOC
   (destination host, stored key). Distinguish "can exec" from "demonstrably execs X".

Keep this distinct from the Rust self-audit catalog — different sources, different
sinks, different goal (characterize the adversary's code vs. harden ours).
