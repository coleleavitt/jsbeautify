(function () {
  "use strict";

  const state = {
    items: [],
    pageSnapshot: null,
    scanStartedAt: null,
    scanFinishedAt: null,
    scanOptions: null,
    resourceErrors: [],
    serviceWorkerAssets: [],
    frameworkInventory: [],
    logs: [],
    // Passive network capture: the ACTUAL response bodies the page received,
    // recorded from devtools.network.onRequestFinished + entry.getContent().
    // Keyed by URL. Preferred over re-fetching so we see exactly what loaded
    // (cache-busted/nonce'd/auth/one-time URLs that a re-fetch would miss).
    network: {
      byUrl: new Map(),
      totalBytes: 0,
      captured: 0,
      truncated: false,
    },
  };

  // Memory bounds for the network-capture buffer.
  const NETWORK_MAX_ENTRY_BYTES = 8 * 1024 * 1024;
  const NETWORK_MAX_TOTAL_BYTES = 192 * 1024 * 1024;

  const els = {
    targetLabel: document.getElementById("targetLabel"),
    installHookButton: document.getElementById("installHookButton"),
    scanButton: document.getElementById("scanButton"),
    dumpFolderButton: document.getElementById("dumpFolderButton"),
    downloadButton: document.getElementById("downloadButton"),
    sendBackendButton: document.getElementById("sendBackendButton"),
    backendUrl: document.getElementById("backendUrl"),
    clearButton: document.getElementById("clearButton"),
    includeGenerated: document.getElementById("includeGenerated"),
    resolveSourceMaps: document.getElementById("resolveSourceMaps"),
    fetchMissingSources: document.getElementById("fetchMissingSources"),
    prettyFallback: document.getElementById("prettyFallback"),
    credentialedFetches: document.getElementById("credentialedFetches"),
    deepExternalOrigins: document.getElementById("deepExternalOrigins"),
    firstPartyFamily: document.getElementById("firstPartyFamily"),
    familyDomains: document.getElementById("familyDomains"),
    deepCrawlMaxCandidates: document.getElementById("deepCrawlMaxCandidates"),
    cfAccessClientId: document.getElementById("cfAccessClientId"),
    cfAccessClientSecret: document.getElementById("cfAccessClientSecret"),
    cfAccessDomains: document.getElementById("cfAccessDomains"),
    reloadButton: document.getElementById("reloadButton"),
    preferNetworkCapture: document.getElementById("preferNetworkCapture"),
    generatedCount: document.getElementById("generatedCount"),
    originalCount: document.getElementById("originalCount"),
    inlineCount: document.getElementById("inlineCount"),
    eventCount: document.getElementById("eventCount"),
    capturedCount: document.getElementById("capturedCount"),
    resourceRows: document.getElementById("resourceRows"),
    log: document.getElementById("log"),
  };

  const textEncoder = new TextEncoder();
  const textDecoder = new TextDecoder();
  const collectableExtensions =
    "(?:mjs|cjs|js|css|json|map|svg|png|jpe?g|gif|webp|avif|ico|woff2?|ttf|otf|eot|wasm|mp4|webm|mp3|wav)";
  const collectableTypes = new Set([
    "script",
    "stylesheet",
    "json",
    "source-map",
    "font",
    "image",
    "wasm",
    "media",
  ]);
  const textTypes = new Set(["script", "stylesheet", "json", "source-map", "text", "html"]);

  function numericOption(value, fallback, min, max) {
    const parsed = Number.parseInt(String(value || ""), 10);
    if (!Number.isFinite(parsed)) {
      return fallback;
    }
    return Math.max(min, Math.min(max, parsed));
  }

  function parseDomainList(value) {
    return String(value || "")
      .split(/[,\s]+/)
      .map(domain => domain.trim().toLowerCase().replace(/^\*\./, "").replace(/\.$/, ""))
      .filter(Boolean);
  }

  function parseHostList(value) {
    return String(value || "")
      .split(/[,\s]+/)
      .map(host => {
        const trimmed = host.trim().toLowerCase().replace(/^\*\./, "").replace(/\.$/, "");
        if (!trimmed) {
          return "";
        }
        try {
          return new URL(/^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`).hostname
            .toLowerCase()
            .replace(/\.$/, "");
        } catch (_error) {
          return trimmed.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").split("/")[0];
        }
      })
      .filter(Boolean);
  }

  function options() {
    return {
      includeGenerated: els.includeGenerated.checked,
      resolveSourceMaps: els.resolveSourceMaps.checked,
      fetchMissingSources: els.fetchMissingSources.checked,
      prettyFallback: els.prettyFallback.checked,
      credentialedFetches: els.credentialedFetches.checked,
      deepExternalOrigins: els.deepExternalOrigins.checked,
      firstPartyFamily: els.firstPartyFamily.checked,
      familyDomains: parseDomainList(els.familyDomains.value),
      deepCrawlMaxCandidates: numericOption(els.deepCrawlMaxCandidates.value, 2000, 0, 10000),
      cfAccessClientId: String(els.cfAccessClientId.value || "").trim(),
      cfAccessClientSecret: String(els.cfAccessClientSecret.value || "").trim(),
      cfAccessDomains: parseHostList(els.cfAccessDomains.value),
      preferNetworkCapture: els.preferNetworkCapture ? els.preferNetworkCapture.checked : true,
    };
  }

  function redactedOptions(scanOptions) {
    const safe = { ...(scanOptions || options()) };
    const cfConfigured = Boolean(safe.cfAccessClientId && safe.cfAccessClientSecret);
    delete safe.cfAccessClientId;
    delete safe.cfAccessClientSecret;
    safe.cfAccessConfigured = cfConfigured;
    safe.cfAccessHeaderNames = cfConfigured
      ? ["CF-Access-Client-Id", "CF-Access-Client-Secret"]
      : [];
    return safe;
  }

  function log(message) {
    const line = `[${new Date().toLocaleTimeString()}] ${message}`;
    state.logs.push(line);
    els.log.textContent = state.logs.slice(-250).join("\n");
    els.log.scrollTop = els.log.scrollHeight;
  }

  function setBusy(isBusy) {
    els.scanButton.disabled = isBusy;
    els.installHookButton.disabled = isBusy;
    if (els.reloadButton) {
      els.reloadButton.disabled = isBusy;
    }
    els.dumpFolderButton.disabled = isBusy || state.items.length === 0;
    els.downloadButton.disabled = isBusy || state.items.length === 0;
    els.sendBackendButton.disabled = isBusy || state.items.length === 0;
  }

  // ── Network capture ────────────────────────────────────────────────────────
  // devtools.network.onRequestFinished hands us a HAR entry per finished
  // request; entry.getContent() yields the body the page actually received. We
  // buffer interesting (text/asset) responses keyed by URL so scanning can serve
  // real bytes instead of re-fetching (which may differ or fail).

  function networkContentTypeInteresting(contentType, url) {
    if (/javascript|ecmascript|css|json|wasm|font|image|octet-stream|text\/|application\/.*\+json/i.test(contentType || "")) {
      return true;
    }
    return new RegExp(`\\.${collectableExtensions}(?:[?#]|$)`, "i").test(String(url || ""));
  }

  function captureKey(url) {
    // Index without the fragment; candidate URLs are normalized the same way.
    return String(url || "").split("#")[0];
  }

  function recordNetworkEntry(url, contentType, base64, text, status, statusText, sourceMapHeader) {
    const net = state.network;
    const key = captureKey(url);
    if (net.byUrl.has(key)) {
      return;
    }
    let bytes;
    try {
      bytes = base64 ? base64ToBytes(base64) : textEncoder.encode(text || "");
    } catch (_error) {
      return;
    }
    if (bytes.length > NETWORK_MAX_ENTRY_BYTES) {
      net.truncated = true;
      return;
    }
    if (net.totalBytes + bytes.length > NETWORK_MAX_TOTAL_BYTES) {
      net.truncated = true;
      return;
    }
    net.byUrl.set(key, {
      url,
      contentType: contentType || "",
      status: status == null ? null : status,
      statusText: statusText || "",
      sourceMapHeader: sourceMapHeader || "",
      // Store text decoded lazily by the consumer; keep bytes for binary.
      bytes,
      isBase64: Boolean(base64),
    });
    net.totalBytes += bytes.length;
    net.captured++;
  }

  function lookupNetworkCapture(url) {
    return state.network.byUrl.get(captureKey(url)) || null;
  }

  function installNetworkCapture() {
    const net = browser.devtools && browser.devtools.network;
    if (!net || typeof net.onRequestFinished === "undefined") {
      log("Network capture unavailable (devtools.network.onRequestFinished missing).");
      return;
    }
    net.onRequestFinished.addListener(request => {
      try {
        const resp = request.response || {};
        const content = resp.content || {};
        const url = (request.request && request.request.url) || "";
        if (!url || !/^https?:/i.test(url)) {
          return;
        }
        const contentType = headerValue(resp.headers, "content-type") || content.mimeType || "";
        if (!networkContentTypeInteresting(contentType, url)) {
          return;
        }
        const status = resp.status;
        const statusText = resp.statusText || "";
        const sourceMapHeader = headerValue(resp.headers, "sourcemap") || headerValue(resp.headers, "x-sourcemap");
        const ingest = (body, encoding) => {
          if (body == null) {
            return;
          }
          if (String(encoding || "").toLowerCase() === "base64") {
            recordNetworkEntry(url, contentType, body, null, status, statusText, sourceMapHeader);
          } else {
            recordNetworkEntry(url, contentType, null, body, status, statusText, sourceMapHeader);
          }
          updateCapturedStat();
        };
        // getContent() returns a Promise in Firefox; Chromium passes a callback.
        const maybePromise = request.getContent(ingest);
        if (maybePromise && typeof maybePromise.then === "function") {
          maybePromise.then(result => {
            if (Array.isArray(result)) {
              ingest(result[0], result[1]);
            }
          }, () => {});
        }
      } catch (_error) {
        // Ignore individual capture failures.
      }
    });
    log("Network capture installed; reload or browse the page to record loaded responses.");
  }

  function headerValue(headers, name) {
    for (const header of headers || []) {
      if (header && String(header.name || "").toLowerCase() === name) {
        return String(header.value == null ? "" : header.value);
      }
    }
    return "";
  }

  function updateCapturedStat() {
    if (els.capturedCount) {
      els.capturedCount.textContent = String(state.network.captured);
    }
  }

  function bytesForContent(content) {
    if (content instanceof Uint8Array) {
      return content;
    }
    if (content instanceof ArrayBuffer) {
      return new Uint8Array(content);
    }
    return textEncoder.encode(String(content || ""));
  }

  function byteLength(content) {
    return bytesForContent(content).length;
  }

  function isProbablyMinified(text, kind) {
    if (!text || text.length < 800) {
      return false;
    }
    const sample = text.slice(0, 12000);
    const lines = sample.split(/\r\n?|\n/);
    const averageLine = sample.length / Math.max(1, lines.length);
    if (kind === "stylesheet") {
      return averageLine > 420 || /}\s*[.#@a-zA-Z]/.test(sample);
    }
    return averageLine > 500 || /;[A-Za-z_$][\w$]*=/.test(sample);
  }

  function inferType(url, resourceType) {
    const hinted = String(resourceType || "").toLowerCase();
    const value = String(url || "");
    if (
      hinted === "script" ||
      hinted === "javascript" ||
      hinted === "module" ||
      hinted === "worker" ||
      /\.(mjs|cjs|js)(?:[?#]|$)/i.test(value)
    ) {
      return "script";
    }
    if (hinted === "stylesheet" || hinted === "style" || hinted === "css" || /\.css(?:[?#]|$)/i.test(value)) {
      return "stylesheet";
    }
    if (hinted === "source-map" || /\.map(?:[?#]|$)/i.test(value)) {
      return "source-map";
    }
    if (hinted === "font" || /\.(woff2?|ttf|otf|eot)(?:[?#]|$)/i.test(value)) {
      return "font";
    }
    if (hinted === "image" || /\.(svg|png|jpe?g|gif|webp|avif|ico)(?:[?#]|$)/i.test(value)) {
      return "image";
    }
    if (hinted === "wasm" || /\.wasm(?:[?#]|$)/i.test(value)) {
      return "wasm";
    }
    if (hinted === "media" || /\.(mp4|webm|mp3|wav)(?:[?#]|$)/i.test(value)) {
      return "media";
    }
    if (hinted === "json" || /\.json(?:[?#]|$)/i.test(value)) {
      return "json";
    }
    return hinted || "other";
  }

  function isCollectableType(type) {
    return collectableTypes.has(type);
  }

  function isTextResource(type, contentType) {
    if (textTypes.has(type)) {
      return true;
    }
    return /(?:^text\/|javascript|ecmascript|json|css|xml|svg)/i.test(contentType || "");
  }

  function isInterestingResource(resource) {
    const type = inferType(resource.url || resource.name || "", resource.type || resource.initiatorType);
    return isCollectableType(type);
  }

  function addItem(item) {
    const id = state.items.length + 1;
    const content = item.content == null ? "" : item.content;
    const text = typeof content === "string" ? content : "";
    const stored = {
      id,
      bytes: byteLength(content),
      minified: isProbablyMinified(text, item.resourceType),
      capturedAt: new Date().toISOString(),
      ...item,
      content,
    };
    state.items.push(stored);
    return stored;
  }

  function errorDetails(error) {
    return {
      status: error && error.status != null ? error.status : null,
      statusText: error && error.statusText ? error.statusText : "",
      contentType: error && error.contentType ? error.contentType : "",
      finalURL: error && error.finalURL ? error.finalURL : "",
      bodyPrefix: error && error.bodyPrefix ? error.bodyPrefix : "",
      credentialMode: error && error.credentialMode ? error.credentialMode : "",
      requestHeaderNames: Array.isArray(error && error.requestHeaderNames)
        ? error.requestHeaderNames.slice()
        : [],
    };
  }

  function addResourceError(error) {
    const record = {
      at: new Date().toISOString(),
      phase: error.phase || "unknown",
      url: error.url || "",
      generatedUrl: error.generatedUrl || null,
      resourceType: error.resourceType || null,
      source: error.source || null,
      message: error.message || "",
      ...errorDetails(error),
    };
    state.resourceErrors.push(record);
    return record;
  }

  function upsertServiceWorkerAsset(asset, patch = {}) {
    if (!asset || !asset.url) {
      return null;
    }
    let record = state.serviceWorkerAssets.find(item => item.url === asset.url);
    if (!record) {
      record = {
        url: asset.url,
        type: asset.type || inferType(asset.url, ""),
        discoveredFrom: asset.discoveredFrom || null,
        precacheRevision: asset.precacheRevision || null,
        precacheIntegrity: asset.precacheIntegrity || null,
        present: false,
        status: "discovered",
        httpStatus: null,
        contentType: null,
        finalUrl: null,
        credentialMode: null,
        requestHeaderNames: [],
        errorPhase: null,
        errorMessage: null,
      };
      state.serviceWorkerAssets.push(record);
    }
    Object.assign(record, patch);
    return record;
  }

  async function devtoolsEvalFunction(fn, args) {
    const expression = `(${fn.toString()}).apply(null, ${JSON.stringify(args || [])})`;
    const [result, exceptionInfo] = await browser.devtools.inspectedWindow.eval(expression);
    if (exceptionInfo) {
      const details = Array.isArray(exceptionInfo.details)
        ? ` (${exceptionInfo.details.join(", ")})`
        : "";
      throw new Error(
        exceptionInfo.description ||
          exceptionInfo.value ||
          exceptionInfo.code ||
          `inspectedWindow.eval failed${details}`
      );
    }
    return result;
  }

  function installRuntimeHookInPage() {
    const key = "__mindemonRuntimeDump";
    if (window[key] && window[key].version) {
      return {
        installed: false,
        alreadyInstalled: true,
        eventCount: window[key].events.length,
      };
    }

    const store = {
      version: "0.1.0",
      installedAt: new Date().toISOString(),
      events: [],
      maxEvents: 10000,
      restores: [],
      observers: [],
    };
    const responseURLMap = typeof WeakMap === "function" ? new WeakMap() : null;
    const bufferURLMap = typeof WeakMap === "function" ? new WeakMap() : null;
    const blobRecords = typeof WeakMap === "function" ? new WeakMap() : null;
    const objectURLRecords = new Map();

    function interestingURL(url) {
      return /\.(mjs|cjs|js|css|json|map|wasm|woff2?|ttf|otf|svg|png|jpe?g|gif|webp|avif|ico|mp4|webm|mp3|wav)(?:[?#]|$)/i.test(String(url || ""));
    }

    function resolveAbsoluteURL(url) {
      try {
        return new URL(String(url || ""), location.href).href;
      } catch (_error) {
        return String(url || "");
      }
    }

    function push(kind, payload) {
      const event = {
        kind,
        at: new Date().toISOString(),
        href: location.href,
        payload,
      };
      store.events.push(event);
      if (store.events.length > store.maxEvents) {
        store.events.splice(0, store.events.length - store.maxEvents);
      }
    }

    function decodeRuntimeReferenceURL(rawURL) {
      return String(rawURL || "")
        .replace(/\\u002[fF]/g, "/")
        .replace(/\\x2[fF]/g, "/")
        .replace(/\\\//g, "/")
        .replace(/&amp;/g, "&")
        .trim();
    }

    function runtimeReferenceType(url) {
      const value = String(url || "");
      if (/\.(?:mjs|cjs|js)(?:[?#]|$)/i.test(value)) {
        return "script";
      }
      if (/\.css(?:[?#]|$)/i.test(value)) {
        return "stylesheet";
      }
      if (/\.wasm(?:[?#]|$)/i.test(value)) {
        return "wasm";
      }
      if (/\.(?:woff2?|ttf|otf|eot)(?:[?#]|$)/i.test(value)) {
        return "font";
      }
      if (/\.(?:svg|png|jpe?g|gif|webp|avif|ico)(?:[?#]|$)/i.test(value)) {
        return "image";
      }
      if (/\.(?:mp4|webm|mp3|wav)(?:[?#]|$)/i.test(value)) {
        return "media";
      }
      if (/\.json(?:[?#]|$)/i.test(value)) {
        return "json";
      }
      if (/\.map(?:[?#]|$)/i.test(value)) {
        return "source-map";
      }
      return "other";
    }

    function extractRuntimeReferences(text) {
      const source = String(text || "");
      if (!source) {
        return [];
      }
      const references = [];
      const seen = new Set();
      const extension = "(?:mjs|cjs|js|css|json|map|svg|png|jpe?g|gif|webp|avif|ico|woff2?|ttf|otf|eot|wasm|mp4|webm|mp3|wav)";
      const patterns = [
        {
          pattern: new RegExp(String.raw`importScripts\(\s*["'\`]([^"'\`]+\.${extension}(?:[?#][^"'\`]*)?)["'\`]`, "gi"),
          source: "runtime-importscripts",
        },
        {
          pattern: new RegExp(String.raw`import\(\s*["'\`]([^"'\`]+\.${extension}(?:[?#][^"'\`]*)?)["'\`]`, "gi"),
          source: "runtime-dynamic-import",
        },
        {
          pattern: new RegExp(String.raw`new\s+URL\(\s*["'\`]([^"'\`]+\.${extension}(?:[?#][^"'\`]*)?)["'\`]\s*,\s*import\.meta\.url\s*\)`, "gi"),
          source: "runtime-import-meta-url",
        },
        {
          pattern: new RegExp(String.raw`["'\`]([^"'\`\s<>]{1,2048}\.${extension}(?:[?#][^"'\`\s<>)]*)?)["'\`]`, "gi"),
          source: "runtime-quoted-asset",
        },
      ];
      for (const entry of patterns) {
        let match;
        while ((match = entry.pattern.exec(source))) {
          const raw = decodeRuntimeReferenceURL(match[1]);
          if (!raw || seen.has(raw)) {
            continue;
          }
          seen.add(raw);
          references.push({
            url: raw,
            type: runtimeReferenceType(raw),
            source: entry.source,
          });
        }
      }
      return references.slice(0, 100);
    }

    function isProbablyJavaScriptBlob(type, text) {
      return /javascript|ecmascript|text\/plain|application\/octet-stream/i.test(type || "") ||
        /\b(?:importScripts|import\s*\(|new\s+Worker|onmessage|postMessage|self\.)\b/.test(text || "");
    }

    function textFromBlobParts(parts) {
      let out = "";
      for (const part of parts || []) {
        if (typeof part === "string") {
          out += part;
        } else if (part instanceof ArrayBuffer) {
          if (part.byteLength > 262144) {
            return "";
          }
          out += new TextDecoder().decode(new Uint8Array(part));
        } else if (ArrayBuffer.isView(part)) {
          if (part.byteLength > 262144) {
            return "";
          }
          out += new TextDecoder().decode(new Uint8Array(part.buffer, part.byteOffset, part.byteLength));
        } else {
          return "";
        }
        if (out.length > 262144) {
          return "";
        }
      }
      return out;
    }

    function wasmURLForSource(source) {
      if (!bufferURLMap || !source) {
        return "";
      }
      if (bufferURLMap.has(source)) {
        return bufferURLMap.get(source);
      }
      if (ArrayBuffer.isView(source) && bufferURLMap.has(source.buffer)) {
        return bufferURLMap.get(source.buffer);
      }
      return "";
    }

    function looksLikeWasmBytes(source) {
      let view = null;
      if (source instanceof ArrayBuffer) {
        view = new Uint8Array(source, 0, Math.min(4, source.byteLength));
      } else if (ArrayBuffer.isView(source)) {
        view = new Uint8Array(source.buffer, source.byteOffset, Math.min(4, source.byteLength));
      }
      return Boolean(view && view.length >= 4 && view[0] === 0x00 && view[1] === 0x61 && view[2] === 0x73 && view[3] === 0x6d);
    }

    function nodeInfo(node) {
      if (!node || node.nodeType !== 1) {
        return null;
      }
      const tagName = node.tagName.toLowerCase();
      if (tagName === "script") {
        return {
          tagName,
          src: node.src || "",
          type: node.type || "",
          async: Boolean(node.async),
          defer: Boolean(node.defer),
          textLength: node.src ? 0 : (node.textContent || "").length,
        };
      }
      if (tagName === "link") {
        const rel = node.rel || "";
        const href = node.href || "";
        if (/stylesheet/i.test(rel) || interestingURL(href)) {
          return {
            tagName,
            rel,
            href,
            as: node.as || "",
            type: node.type || "",
          };
        }
      }
      if (tagName === "style") {
        return {
          tagName,
          textLength: (node.textContent || "").length,
        };
      }
      return null;
    }

    function recordNode(node, reason) {
      const info = nodeInfo(node);
      if (info) {
        push("dom-" + reason, info);
      }
      if (node && node.querySelectorAll) {
        for (const child of node.querySelectorAll("script,link,style")) {
          const childInfo = nodeInfo(child);
          if (childInfo) {
            push("dom-" + reason, childInfo);
          }
        }
      }
    }

    for (const node of document.querySelectorAll("script,link,style")) {
      recordNode(node, "initial");
    }

    try {
      for (const entry of performance.getEntriesByType("resource")) {
        if (interestingURL(entry.name) || ["script", "css", "link"].includes(entry.initiatorType)) {
          push("performance-resource", {
            name: entry.name,
            initiatorType: entry.initiatorType,
            startTime: entry.startTime,
            duration: entry.duration,
            transferSize: entry.transferSize,
            encodedBodySize: entry.encodedBodySize,
            decodedBodySize: entry.decodedBodySize,
          });
        }
      }
    } catch (_error) {
      push("performance-error", { message: "Unable to read performance entries" });
    }

    try {
      const observer = new MutationObserver(records => {
        for (const record of records) {
          for (const node of record.addedNodes) {
            recordNode(node, "added");
          }
        }
      });
      observer.observe(document.documentElement || document, {
        childList: true,
        subtree: true,
      });
      store.observers.push(observer);
    } catch (error) {
      push("mutation-error", { message: String(error && error.message ? error.message : error) });
    }

    try {
      const perfObserver = new PerformanceObserver(list => {
        for (const entry of list.getEntries()) {
          if (interestingURL(entry.name) || ["script", "css", "link"].includes(entry.initiatorType)) {
            push("performance-resource", {
              name: entry.name,
              initiatorType: entry.initiatorType,
              startTime: entry.startTime,
              duration: entry.duration,
              transferSize: entry.transferSize,
              encodedBodySize: entry.encodedBodySize,
              decodedBodySize: entry.decodedBodySize,
            });
          }
        }
      });
      perfObserver.observe({ entryTypes: ["resource"] });
      store.observers.push(perfObserver);
    } catch (error) {
      push("performance-observer-error", {
        message: String(error && error.message ? error.message : error),
      });
    }

    if (typeof window.fetch === "function") {
      const nativeFetch = window.fetch;
      window.fetch = function mindemonFetch(input, init) {
        const url = typeof input === "string" ? input : input && input.url;
        const startedAt = Date.now();
        const result = nativeFetch.apply(this, arguments);
        Promise.resolve(result).then(
          response => {
            if (responseURLMap && response) {
              try {
                responseURLMap.set(response, resolveAbsoluteURL(response.url || url || ""));
              } catch (_error) {
                // Ignore weak-map bookkeeping errors.
              }
            }
            const contentType = response.headers && response.headers.get("content-type");
            if (interestingURL(url) || /javascript|css|source-map/i.test(contentType || "")) {
              push("fetch", {
                url,
                status: response.status,
                contentType,
                durationMs: Date.now() - startedAt,
              });
            }
          },
          error => {
            if (interestingURL(url)) {
              push("fetch-error", {
                url,
                message: String(error && error.message ? error.message : error),
              });
            }
          }
        );
        return result;
      };
      store.restores.push(() => {
        window.fetch = nativeFetch;
      });
    }

    if (window.Response && window.Response.prototype && typeof window.Response.prototype.arrayBuffer === "function") {
      const nativeArrayBuffer = window.Response.prototype.arrayBuffer;
      const nativeClone = typeof window.Response.prototype.clone === "function"
        ? window.Response.prototype.clone
        : null;
      if (nativeClone) {
        window.Response.prototype.clone = function mindemonResponseClone() {
          const cloned = nativeClone.apply(this, arguments);
          try {
            if (responseURLMap && responseURLMap.has(this)) {
              responseURLMap.set(cloned, responseURLMap.get(this));
            }
          } catch (_error) {
            // Ignore weak-map bookkeeping errors.
          }
          return cloned;
        };
      }
      window.Response.prototype.arrayBuffer = function mindemonResponseArrayBuffer() {
        const response = this;
        const result = nativeArrayBuffer.apply(this, arguments);
        Promise.resolve(result).then(
          buffer => {
            try {
              const url = responseURLMap && responseURLMap.get(response)
                ? responseURLMap.get(response)
                : (response && response.url ? response.url : "");
              if (url && bufferURLMap && buffer instanceof ArrayBuffer) {
                bufferURLMap.set(buffer, resolveAbsoluteURL(url));
              }
              if (url && looksLikeWasmBytes(buffer)) {
                push("wasm", {
                  url: resolveAbsoluteURL(url),
                  method: "response.arrayBuffer",
                  byteLength: buffer.byteLength || 0,
                });
              }
            } catch (_error) {
              // Ignore instrumentation errors.
            }
          },
          () => {}
        );
        return result;
      };
      store.restores.push(() => {
        window.Response.prototype.arrayBuffer = nativeArrayBuffer;
        if (nativeClone) {
          window.Response.prototype.clone = nativeClone;
        }
      });
    }

    if (window.XMLHttpRequest && window.XMLHttpRequest.prototype) {
      const proto = window.XMLHttpRequest.prototype;
      const nativeOpen = proto.open;
      const nativeSend = proto.send;
      proto.open = function mindemonXHROpen(method, url) {
        this.__mindemonRequest = { method, url, startedAt: 0 };
        return nativeOpen.apply(this, arguments);
      };
      proto.send = function mindemonXHRSend() {
        const request = this.__mindemonRequest;
        if (request) {
          request.startedAt = Date.now();
          this.addEventListener("loadend", function () {
            const contentType = this.getResponseHeader("content-type") || "";
            if (interestingURL(request.url) || /javascript|css|source-map/i.test(contentType)) {
              push("xhr", {
                method: request.method,
                url: request.url,
                status: this.status,
                contentType,
                durationMs: Date.now() - request.startedAt,
              });
            }
          }, { once: true });
        }
        return nativeSend.apply(this, arguments);
      };
      store.restores.push(() => {
        proto.open = nativeOpen;
        proto.send = nativeSend;
      });
    }

    if (typeof window.Blob === "function" && blobRecords) {
      const NativeBlob = window.Blob;
      try {
        window.Blob = new Proxy(NativeBlob, {
          construct(target, args) {
            const blob = Reflect.construct(target, args);
            try {
              const parts = Array.isArray(args[0]) ? args[0] : [];
              const opts = args[1] || {};
              const text = textFromBlobParts(parts);
              const type = String((opts && opts.type) || blob.type || "");
              if (text && isProbablyJavaScriptBlob(type, text)) {
                blobRecords.set(blob, {
                  type,
                  size: blob.size || text.length,
                  text,
                  textLength: text.length,
                  textPrefix: text.replace(/\s+/g, " ").trim().slice(0, 500),
                  references: extractRuntimeReferences(text),
                });
              }
            } catch (_error) {
              // Ignore blob inspection failures.
            }
            return blob;
          },
        });
        store.restores.push(() => {
          window.Blob = NativeBlob;
        });
      } catch (_error) {
        // Ignore if Blob is not writable.
      }
    }

    if (window.URL && typeof window.URL.createObjectURL === "function") {
      const nativeCreateObjectURL = window.URL.createObjectURL;
      const nativeRevokeObjectURL = typeof window.URL.revokeObjectURL === "function"
        ? window.URL.revokeObjectURL
        : null;
      window.URL.createObjectURL = function mindemonCreateObjectURL(object) {
        const url = nativeCreateObjectURL.apply(this, arguments);
        try {
          const record = blobRecords && blobRecords.get(object);
          if (record) {
            const payload = {
              url,
              type: record.type,
              size: record.size,
              textLength: record.textLength,
              textPrefix: record.textPrefix,
              text: record.text,
              references: record.references,
            };
            objectURLRecords.set(url, payload);
            push("blob-url", payload);
          }
        } catch (_error) {
          // Ignore object URL inspection failures.
        }
        return url;
      };
      if (nativeRevokeObjectURL) {
        window.URL.revokeObjectURL = function mindemonRevokeObjectURL(url) {
          try {
            objectURLRecords.delete(String(url || ""));
          } catch (_error) {
            // Ignore object URL bookkeeping failures.
          }
          return nativeRevokeObjectURL.apply(this, arguments);
        };
      }
      store.restores.push(() => {
        window.URL.createObjectURL = nativeCreateObjectURL;
        if (nativeRevokeObjectURL) {
          window.URL.revokeObjectURL = nativeRevokeObjectURL;
        }
      });
    }

    function recordWorker(scriptURL, opts, workerType) {
      try {
        const raw = scriptURL && scriptURL.href ? scriptURL.href : scriptURL;
        const text = String(raw || "");
        // Blob/data worker bodies aren't fetchable same-origin assets; record but flag.
        const inlineWorker = /^blob:|^data:/i.test(text);
        const blobRecord = inlineWorker ? objectURLRecords.get(text) : null;
        push("worker", {
          url: inlineWorker ? text : resolveAbsoluteURL(text),
          workerType,
          moduleType: opts && opts.type === "module" ? "module" : "classic",
          inline: inlineWorker,
          blobTextLength: blobRecord ? blobRecord.textLength : 0,
          blobTextPrefix: blobRecord ? blobRecord.textPrefix : "",
          references: blobRecord ? blobRecord.references : [],
        });
      } catch (_error) {
        // Ignore instrumentation errors.
      }
    }

    function wrapWorkerConstructor(Native, workerType) {
      // Proxy preserves prototype chain, instanceof, and all constructor args.
      return new Proxy(Native, {
        construct(target, args) {
          recordWorker(args[0], args[1], workerType);
          return Reflect.construct(target, args, target);
        },
      });
    }

    if (typeof window.Worker === "function") {
      const NativeWorker = window.Worker;
      try {
        window.Worker = wrapWorkerConstructor(NativeWorker, "dedicated");
        store.restores.push(() => {
          window.Worker = NativeWorker;
        });
      } catch (_error) {
        // Ignore if Worker is not writable.
      }
    }

    if (typeof window.SharedWorker === "function") {
      const NativeSharedWorker = window.SharedWorker;
      try {
        window.SharedWorker = wrapWorkerConstructor(NativeSharedWorker, "shared");
        store.restores.push(() => {
          window.SharedWorker = NativeSharedWorker;
        });
      } catch (_error) {
        // Ignore if SharedWorker is not writable.
      }
    }

    if (navigator.serviceWorker && typeof navigator.serviceWorker.register === "function") {
      const swContainer = navigator.serviceWorker;
      const nativeRegister = swContainer.register.bind(swContainer);
      try {
        swContainer.register = function mindemonRegister(scriptURL, opts) {
          try {
            push("service-worker", { url: resolveAbsoluteURL(scriptURL), scope: opts && opts.scope ? opts.scope : "" });
          } catch (_error) {
            // Ignore instrumentation errors.
          }
          return nativeRegister(scriptURL, opts);
        };
        store.restores.push(() => {
          swContainer.register = nativeRegister;
        });
      } catch (_error) {
        // Ignore if register is not writable.
      }
      try {
        if (swContainer.controller && swContainer.controller.scriptURL) {
          push("service-worker", { url: swContainer.controller.scriptURL, scope: "", active: true });
        }
      } catch (_error) {
        // Ignore controller read errors.
      }
    }

    if (window.WebAssembly) {
      for (const method of ["instantiateStreaming", "compileStreaming"]) {
        const nativeWasm = window.WebAssembly[method];
        if (typeof nativeWasm !== "function") {
          continue;
        }
        window.WebAssembly[method] = function mindemonWasm(source) {
          Promise.resolve(source).then(
            response => {
              try {
                const url = response && response.url
                  ? response.url
                  : (typeof source === "string" ? source : "");
                if (url) {
                  push("wasm", { url: resolveAbsoluteURL(url), method });
                }
              } catch (_error) {
                // Ignore instrumentation errors.
              }
            },
            () => {}
          );
          return nativeWasm.apply(this, arguments);
        };
        store.restores.push(((m, fn) => () => {
          window.WebAssembly[m] = fn;
        })(method, nativeWasm));
      }
      for (const method of ["instantiate", "compile"]) {
        const nativeWasm = window.WebAssembly[method];
        if (typeof nativeWasm !== "function") {
          continue;
        }
        window.WebAssembly[method] = function mindemonWasmBuffer(source) {
          try {
            const url = wasmURLForSource(source);
            if (url || looksLikeWasmBytes(source)) {
              push("wasm", {
                url: url ? resolveAbsoluteURL(url) : "",
                method,
                byteLength: source && source.byteLength ? source.byteLength : 0,
                inline: !url,
              });
            }
          } catch (_error) {
            // Ignore instrumentation errors.
          }
          return nativeWasm.apply(this, arguments);
        };
        store.restores.push(((m, fn) => () => {
          window.WebAssembly[m] = fn;
        })(method, nativeWasm));
      }
    }

    store.stop = function stopMindemonRuntimeDump() {
      for (const observer of store.observers.splice(0)) {
        try {
          observer.disconnect();
        } catch (_error) {
          // Ignore teardown errors.
        }
      }
      for (const restore of store.restores.splice(0)) {
        try {
          restore();
        } catch (_error) {
          // Ignore teardown errors.
        }
      }
      push("hook-stopped", {});
    };

    window[key] = store;
    push("hook-installed", { href: location.href });

    return {
      installed: true,
      alreadyInstalled: false,
      eventCount: store.events.length,
    };
  }

  function collectPageSnapshotInPage() {
    const hook = window.__mindemonRuntimeDump;

    function parseImportMaps(doc) {
      const maps = [];
      for (const node of doc.querySelectorAll('script[type="importmap"]')) {
        try {
          const parsed = JSON.parse(node.textContent || "{}");
          maps.push({
            imports: parsed.imports || {},
            scopes: parsed.scopes || {},
          });
        } catch (error) {
          maps.push({ parseError: String(error && error.message ? error.message : error) });
        }
      }
      return maps;
    }

    function dataScripts(doc) {
      const out = [];
      const selector = 'script[type="application/json"],script[type="application/ld+json"],script[type="speculationrules"],script[type="application/importmap+json"]';
      let index = 0;
      for (const node of doc.querySelectorAll(selector)) {
        out.push({
          index: index++,
          scriptType: node.type || "",
          id: node.id || "",
          text: capText(node.textContent || "", CAP_INLINE_TEXT, "inlineText"),
        });
      }
      return out;
    }

    function hydrationGlobals() {
      const names = [
        "__OMELETTE_ME__", "__NEXT_DATA__", "__NEXT_F", "__remixContext",
        "__remixManifest", "__remixRouteModules", "__APOLLO_STATE__",
        "__INITIAL_STATE__", "__PRELOADED_STATE__", "__NUXT__", "__SVELTEKIT_DATA__",
        "__sveltekit", "__INITIAL_DATA__", "__data", "__STATE__", "__REDUX_STATE__",
      ];
      const found = {};
      for (const name of names) {
        try {
          if (Object.prototype.hasOwnProperty.call(window, name) && window[name] != null) {
            const value = window[name];
            const type = typeof value;
            if (type === "object" || type === "string") {
              found[name] = value;
            }
          }
        } catch (_error) {
          // Ignore globals that throw on access.
        }
      }
      return found;
    }

    // Per-field caps so the whole snapshot can't blow the inspectedWindow.eval
    // JSON bridge (which silently truncates / OOMs on very large returns).
    const CAP_INLINE_TEXT = 512 * 1024;      // per inline <script>/<style>
    const CAP_RULES_TEXT = 256 * 1024;       // per inline (no-href) stylesheet
    const CAP_TOTAL_INLINE = 16 * 1024 * 1024; // total inline text budget
    const MAX_EVENTS = 10000;                // runtime-hook events returned
    const truncation = { inlineText: false, rulesText: false, total: false, frames: false, events: false };
    let inlineBudget = CAP_TOTAL_INLINE;

    function capText(text, cap, flagKey) {
      const str = String(text || "");
      let out = str;
      if (str.length > cap) {
        out = str.slice(0, cap);
        truncation[flagKey] = true;
      }
      if (out.length > inlineBudget) {
        out = out.slice(0, Math.max(0, inlineBudget));
        truncation.total = true;
      }
      inlineBudget -= out.length;
      return out;
    }

    function sheetInfo(sheet, index) {
      const owner = sheet.ownerNode;
      const info = {
        index,
        href: sheet.href || "",
        disabled: Boolean(sheet.disabled),
        ownerTag: owner && owner.tagName ? owner.tagName.toLowerCase() : "",
        ownerText: "",
        rulesText: "",
        rulesError: "",
        rulesDeferred: false,
      };
      const isInline = !sheet.href;
      if (owner && owner.tagName && owner.tagName.toLowerCase() === "style") {
        info.ownerText = capText(owner.textContent || "", CAP_INLINE_TEXT, "inlineText");
      }
      // For href-backed sheets, do NOT read cssRules: cross-origin sheets throw,
      // and same-origin rulesText just duplicates bytes we fetch/capture anyway
      // (and bloats the eval bridge). Inline (no-href) sheets are the only place
      // the rules aren't otherwise recoverable, so serialize just those.
      if (isInline) {
        try {
          const rules = Array.from(sheet.cssRules || []);
          info.rulesText = capText(rules.map(rule => rule.cssText).join("\n"), CAP_RULES_TEXT, "rulesText");
        } catch (error) {
          info.rulesError = String(error && error.message ? error.message : error);
        }
      } else {
        info.rulesDeferred = true;
      }
      return info;
    }

    // Collect script/link/style/srcset/data from one document. Walked for the
    // top document and each same-origin iframe (framePath identifies the frame).
    function collectDocument(doc, framePath) {
      return {
        framePath,
        scripts: Array.from(doc.scripts).map((script, index) => ({
          index,
          framePath,
          src: script.src || "",
          type: script.type || "",
          async: Boolean(script.async),
          defer: Boolean(script.defer),
          text: script.src ? "" : capText(script.textContent || "", CAP_INLINE_TEXT, "inlineText"),
        })),
        links: Array.from(doc.querySelectorAll("link")).map((link, index) => ({
          index,
          framePath,
          href: link.href || "",
          rel: link.rel || "",
          as: link.as || "",
          type: link.type || "",
          imagesrcset: (link.getAttribute && link.getAttribute("imagesrcset")) || "",
        })),
        styles: Array.from(doc.querySelectorAll("style")).map((style, index) => ({
          index,
          framePath,
          text: capText(style.textContent || "", CAP_INLINE_TEXT, "inlineText"),
        })),
        srcsets: Array.from(doc.querySelectorAll("img[srcset],source[srcset]")).map((node, index) => ({
          index,
          framePath,
          srcset: node.getAttribute("srcset") || "",
          src: node.currentSrc || node.src || "",
        })),
        styleSheets: Array.from(doc.styleSheets).map((sheet, index) => {
          const info = sheetInfo(sheet, index);
          info.framePath = framePath;
          return info;
        }),
        importMaps: parseImportMaps(doc),
        dataScripts: dataScripts(doc),
      };
    }

    // Recursively walk frames. Same-origin frames are merged into the main
    // collections (tagged with framePath); cross-origin frames are recorded as
    // URL-only entries so their src can still be fetched/captured.
    const MAX_FRAME_DEPTH = 5;
    const MAX_FRAMES = 100;
    const frames = [];
    function walkFrames(doc, parentPath, depth, agg) {
      if (depth > MAX_FRAME_DEPTH) {
        return;
      }
      const iframes = Array.from(doc.querySelectorAll("iframe,frame"));
      for (let i = 0; i < iframes.length; i++) {
        if (frames.length >= MAX_FRAMES) {
          truncation.frames = true;
          return;
        }
        const iframe = iframes[i];
        const framePath = `${parentPath}/${i}`;
        const src = iframe.src || "";
        let childDoc = null;
        let accessible = false;
        try {
          childDoc = iframe.contentDocument;
          accessible = Boolean(childDoc);
        } catch (_error) {
          accessible = false; // cross-origin
        }
        frames.push({ framePath, src, accessible });
        if (accessible && childDoc) {
          const collected = collectDocument(childDoc, framePath);
          agg.scripts.push(...collected.scripts);
          agg.links.push(...collected.links);
          agg.styles.push(...collected.styles);
          agg.srcsets.push(...collected.srcsets);
          agg.styleSheets.push(...collected.styleSheets);
          walkFrames(childDoc, framePath, depth + 1, agg);
        }
      }
    }

    const top = collectDocument(document, "");
    walkFrames(document, "", 0, top);

    const events = hook ? hook.events : [];
    if (events.length > MAX_EVENTS) {
      truncation.events = true;
    }

    return {
      href: location.href,
      title: document.title,
      collectedAt: new Date().toISOString(),
      frames,
      truncation,
      scripts: top.scripts,
      links: top.links,
      srcsets: top.srcsets,
      styles: top.styles,
      styleSheets: top.styleSheets,
      importMaps: top.importMaps,
      dataScripts: top.dataScripts,
      hydrationGlobals: hydrationGlobals(),
      performanceResources: performance.getEntriesByType("resource").map(entry => ({
        name: entry.name,
        initiatorType: entry.initiatorType,
        startTime: entry.startTime,
        duration: entry.duration,
        transferSize: entry.transferSize,
        encodedBodySize: entry.encodedBodySize,
        decodedBodySize: entry.decodedBodySize,
      })),
      runtimeHook: hook ? {
        version: hook.version,
        installedAt: hook.installedAt,
        eventCount: events.length,
        eventsTruncated: truncation.events,
        events: truncation.events ? events.slice(-MAX_EVENTS) : events.slice(),
      } : null,
    };
  }

  function prettyPrint(text, type) {
    if (!text) {
      return "";
    }
    const indentText = "  ";
    let out = "";
    let indent = 0;
    let quote = "";
    let escaped = false;

    function newline() {
      out = out.replace(/[ \t]+$/g, "");
      out += "\n" + indentText.repeat(Math.max(0, indent));
    }

    for (let i = 0; i < text.length; i++) {
      const ch = text[i];
      const prev = text[i - 1];

      if (quote) {
        out += ch;
        if (escaped) {
          escaped = false;
        } else if (ch === "\\") {
          escaped = true;
        } else if (ch === quote) {
          quote = "";
        }
        continue;
      }

      if (ch === "\"" || ch === "'" || ch === "`") {
        quote = ch;
        out += ch;
      } else if (ch === "{" || ch === "[") {
        out += ch;
        indent++;
        newline();
      } else if (ch === "}" || ch === "]") {
        indent--;
        newline();
        out += ch;
      } else if (ch === ";") {
        out += ch;
        newline();
      } else if (type === "stylesheet" && ch === "}") {
        indent--;
        newline();
        out += ch;
        newline();
      } else if (/\s/.test(ch)) {
        if (!/\s/.test(prev || "")) {
          out += " ";
        }
      } else {
        out += ch;
      }
    }
    return out.trim() + "\n";
  }

  // Inline resources live in multiple frames; the top document uses "" so its
  // labels stay backward-compatible, nested frames get a "frameN_N_" prefix.
  function frameLabel(framePath) {
    if (!framePath) {
      return "";
    }
    return "frame" + String(framePath).replace(/^\/+/, "").replace(/\//g, "_") + "_";
  }

  async function collectInlineResources(snapshot) {
    for (const script of snapshot.scripts || []) {
      if (!script.src && script.text) {
        const framePrefix = frameLabel(script.framePath);
        addItem({
          bucket: "inline",
          resourceType: "script",
          url: `inline://script/${framePrefix}${script.index}.js`,
          content: script.text,
          source: "page-inline-script",
          framePath: script.framePath || "",
        });
      }
    }
    for (const style of snapshot.styles || []) {
      if (style.text) {
        const framePrefix = frameLabel(style.framePath);
        addItem({
          bucket: "inline",
          resourceType: "stylesheet",
          url: `inline://style/${framePrefix}${style.index}.css`,
          content: style.text,
          source: "page-inline-style",
          framePath: style.framePath || "",
        });
      }
    }
    // Inline (no-href) stylesheets serialize their rules in styleSheets[].
    for (const sheet of snapshot.styleSheets || []) {
      if (!sheet.href && sheet.rulesText && sheet.ownerTag !== "style") {
        const framePrefix = frameLabel(sheet.framePath);
        addItem({
          bucket: "inline",
          resourceType: "stylesheet",
          url: `inline://sheet/${framePrefix}${sheet.index}.css`,
          content: sheet.rulesText,
          source: "page-inline-stylesheet",
        });
      }
    }
    for (const dataScript of snapshot.dataScripts || []) {
      if (dataScript.text) {
        const suffix = dataScript.id ? `-${sanitizeSegment(dataScript.id)}` : "";
        const framePrefix = frameLabel(dataScript.framePath);
        const label = `data-${framePrefix}${dataScript.index}${suffix}`;
        addItem({
          bucket: "inline",
          resourceType: "json",
          url: `inline://data/${label}.json`,
          content: dataScript.text,
          contentType: dataScript.scriptType,
          source: "page-data-script",
        });
      }
    }
    if (snapshot.hydrationGlobals && Object.keys(snapshot.hydrationGlobals).length) {
      let serialized;
      try {
        serialized = JSON.stringify(snapshot.hydrationGlobals, null, 2);
      } catch (_error) {
        serialized = String(snapshot.hydrationGlobals);
      }
      addItem({
        bucket: "inline",
        resourceType: "json",
        url: "inline://data/hydration-globals.json",
        content: serialized,
        source: "page-hydration-globals",
      });
    }
    const hookEvents = snapshot.runtimeHook ? snapshot.runtimeHook.events || [] : [];
    let blobIndex = 0;
    for (const event of hookEvents) {
      if (event.kind !== "blob-url" || !event.payload || !event.payload.text) {
        continue;
      }
      addItem({
        bucket: "inline",
        resourceType: "script",
        url: `inline://blob/${blobIndex++}.js`,
        content: event.payload.text,
        contentType: event.payload.type || "application/javascript",
        source: "runtime-blob-url",
        discoveredFrom: event.payload.url || null,
      });
    }
  }

  function typeFromContentType(contentType) {
    if (/javascript|ecmascript|json/i.test(contentType || "")) {
      return /json/i.test(contentType) ? "json" : "script";
    }
    if (/css/i.test(contentType || "")) {
      return "stylesheet";
    }
    if (/font|woff|opentype|truetype/i.test(contentType || "")) {
      return "font";
    }
    if (/image/i.test(contentType || "")) {
      return "image";
    }
    if (/wasm/i.test(contentType || "")) {
      return "wasm";
    }
    if (/audio|video/i.test(contentType || "")) {
      return "media";
    }
    return null;
  }

  function pageDirectoryURL(pageURL) {
    try {
      const parsed = new URL(pageURL || location.href);
      if (!parsed.pathname.endsWith("/") && !/\.[^/]+$/.test(parsed.pathname)) {
        parsed.pathname += "/";
      }
      parsed.search = "";
      parsed.hash = "";
      return parsed.href;
    } catch (_error) {
      return location.href;
    }
  }

  function httpOriginForURL(url) {
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        return parsed.origin;
      }
    } catch (_error) {
      // Ignore invalid URLs.
    }
    return "";
  }

  function hostnameForURL(url) {
    try {
      return new URL(url).hostname.toLowerCase();
    } catch (_error) {
      return "";
    }
  }

  // Multi-part public suffixes we may encounter; everything else is treated as a
  // simple two-label eTLD+1. This is a curated subset of the Public Suffix List,
  // not the complete PSL. It covers common ccTLD second levels plus widely-used
  // app-hosting platforms where each subdomain is an independent site (so the
  // first-party-family boundary doesn't accidentally span unrelated tenants).
  const MULTIPART_SUFFIXES = new Set([
    // ccTLD second-level registries
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "net.uk", "ltd.uk", "plc.uk",
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "id.au", "asn.au",
    "co.jp", "or.jp", "ne.jp", "ac.jp", "go.jp", "ad.jp", "ed.jp", "gr.jp",
    "co.nz", "net.nz", "org.nz", "govt.nz", "ac.nz", "geek.nz", "school.nz",
    "co.in", "net.in", "org.in", "gen.in", "firm.in", "ind.in", "gov.in",
    "co.kr", "or.kr", "ne.kr", "go.kr", "re.kr", "pe.kr", "ac.kr",
    "co.za", "org.za", "net.za", "web.za", "gov.za", "ac.za",
    "com.br", "net.br", "org.br", "gov.br", "edu.br",
    "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "ac.cn",
    "com.mx", "org.mx", "net.mx", "gob.mx", "edu.mx",
    "com.tr", "net.tr", "org.tr", "gov.tr", "edu.tr",
    "com.sg", "net.sg", "org.sg", "gov.sg", "edu.sg",
    "com.hk", "net.hk", "org.hk", "gov.hk", "edu.hk", "idv.hk",
    "com.tw", "net.tw", "org.tw", "gov.tw", "edu.tw",
    "com.ar", "net.ar", "org.ar", "gob.ar",
    "co.id", "or.id", "web.id", "ac.id", "go.id",
    "com.my", "net.my", "org.my", "gov.my", "edu.my",
    "com.ph", "net.ph", "org.ph", "gov.ph", "edu.ph",
    "com.pl", "net.pl", "org.pl", "gov.pl", "edu.pl",
    "com.ua", "co.il", "org.il", "ac.il", "gov.il",
    "com.vn", "com.sa", "com.eg", "com.ng", "co.ke", "co.th",
    "com.es", "com.pe", "com.co", "com.ec", "com.uy", "com.ve",
    // Private app-hosting suffixes: each subdomain is a separate site
    "github.io", "githubusercontent.com", "gitlab.io", "bitbucket.io",
    "pages.dev", "workers.dev", "web.app", "firebaseapp.com",
    "vercel.app", "now.sh", "netlify.app", "netlify.com",
    "herokuapp.com", "fly.dev", "render.com", "onrender.com",
    "amazonaws.com", "cloudfront.net", "azurewebsites.net", "azureedge.net",
    "appspot.com", "googleusercontent.com", "translate.goog",
    "blob.core.windows.net", "s3.amazonaws.com",
    "surge.sh", "glitch.me", "repl.co", "replit.dev", "wpcomstaging.com",
  ]);

  function registrableDomain(hostname) {
    // Prefer the canonical shared implementation (lib/patterns.js) when loaded.
    if (typeof MindemonPatterns !== "undefined" && MindemonPatterns.registrableDomain) {
      return MindemonPatterns.registrableDomain(hostname);
    }
    const host = String(hostname || "").toLowerCase().replace(/\.$/, "");
    if (!host || /^[0-9.]+$/.test(host) || host === "localhost") {
      return host;
    }
    const labels = host.split(".");
    if (labels.length <= 2) {
      return host;
    }
    // Longest-suffix match: find the longest known multi-part public suffix that
    // the host ends with, then take one more label as the registrable domain.
    for (let start = 0; start < labels.length - 1; start++) {
      const suffix = labels.slice(start).join(".");
      if (MULTIPART_SUFFIXES.has(suffix) && start >= 1) {
        return labels.slice(start - 1).join(".");
      }
    }
    return labels.slice(-2).join(".");
  }

  function domainInFamily(hostname, familyDomains) {
    const reg = registrableDomain(hostname);
    if (!reg) {
      return false;
    }
    for (const domain of familyDomains) {
      if (reg === domain || hostname === domain || hostname.endsWith("." + domain)) {
        return true;
      }
    }
    return false;
  }

  const DEEP_CRAWL_MAX_DEPTH = 3;
  const DEEP_CRAWL_MAX_CANDIDATES = 2000;

  function normalizeFamilyDomain(domain) {
    const cleaned = String(domain || "").toLowerCase().replace(/^\*\./, "").replace(/\.$/, "");
    if (!cleaned) {
      return "";
    }
    try {
      return registrableDomain(new URL(/^https?:\/\//i.test(cleaned) ? cleaned : `https://${cleaned}`).hostname);
    } catch (_error) {
      return registrableDomain(cleaned);
    }
  }

  function createDeepCrawlScope(pageURL, rootResources, scanOptions) {
    const allowedOrigins = new Set();
    const pageOrigin = httpOriginForURL(pageURL || location.href);
    if (pageOrigin) {
      allowedOrigins.add(pageOrigin);
    }
    for (const resource of rootResources || []) {
      const origin = httpOriginForURL(resource.url);
      if (origin) {
        allowedOrigins.add(origin);
      }
    }

    // First-party family is target-derived by default: the inspected page's
    // registrable domain plus any explicit domains entered in the panel.
    const familyDomains = new Set();
    const pageHost = hostnameForURL(pageURL || location.href);
    if (pageHost) {
      familyDomains.add(registrableDomain(pageHost));
    }
    for (const domain of scanOptions.familyDomains || []) {
      const normalized = normalizeFamilyDomain(domain);
      if (normalized) {
        familyDomains.add(normalized);
      }
    }

    return {
      allowExternalOrigins: !!scanOptions.deepExternalOrigins,
      allowFirstPartyFamily: !!scanOptions.firstPartyFamily,
      familyDomains: Array.from(familyDomains).filter(Boolean),
      useSessionCredentials: !!scanOptions.credentialedFetches,
      preferNetworkCapture: scanOptions.preferNetworkCapture !== false,
      cfAccessClientId: String(scanOptions.cfAccessClientId || ""),
      cfAccessClientSecret: String(scanOptions.cfAccessClientSecret || ""),
      cfAccessDomains: Array.isArray(scanOptions.cfAccessDomains)
        ? scanOptions.cfAccessDomains.slice()
        : [],
      pageOrigin,
      allowedOrigins,
      familyOrigins: new Set(),
      maxDepth: DEEP_CRAWL_MAX_DEPTH,
      maxCandidates: numericOption(
        scanOptions.deepCrawlMaxCandidates,
        DEEP_CRAWL_MAX_CANDIDATES,
        0,
        10000
      ),
      deepAdded: 0,
      capNotified: false,
      skippedExternalCount: 0,
      skippedExternalSeen: new Set(),
      skippedExternalExamples: [],
    };
  }

  function isFamilyOrigin(url, scope) {
    if (!scope || !scope.allowFirstPartyFamily) {
      return false;
    }
    const host = hostnameForURL(url);
    return host ? domainInFamily(host, scope.familyDomains) : false;
  }

  function hostMatchesDomainList(hostname, domains) {
    const host = String(hostname || "").toLowerCase().replace(/\.$/, "");
    for (const domain of domains || []) {
      const cleaned = String(domain || "").toLowerCase().replace(/^\*\./, "").replace(/\.$/, "");
      if (cleaned && (host === cleaned || host.endsWith("." + cleaned))) {
        return true;
      }
    }
    return false;
  }

  function isExplicitCfAccessTarget(url, scope) {
    return Boolean(
      scope &&
        scope.cfAccessClientId &&
        scope.cfAccessClientSecret &&
        Array.isArray(scope.cfAccessDomains) &&
        scope.cfAccessDomains.length &&
        hostMatchesDomainList(hostnameForURL(url), scope.cfAccessDomains)
    );
  }

  function shouldAttachCfAccessHeaders(url, scope) {
    if (!scope || !scope.cfAccessClientId || !scope.cfAccessClientSecret) {
      return false;
    }
    const host = hostnameForURL(url);
    if (!host) {
      return false;
    }
    if (scope.cfAccessDomains.length) {
      return hostMatchesDomainList(host, scope.cfAccessDomains);
    }
    const origin = httpOriginForURL(url);
    return Boolean(
      origin &&
        (origin === scope.pageOrigin || scope.allowedOrigins.has(origin) || isFamilyOrigin(url, scope))
    );
  }

  function requestHeadersForFetch(url, scope) {
    if (!shouldAttachCfAccessHeaders(url, scope)) {
      return {};
    }
    return {
      "CF-Access-Client-Id": scope.cfAccessClientId,
      "CF-Access-Client-Secret": scope.cfAccessClientSecret,
    };
  }

  function requestHeaderNames(headers) {
    return Object.keys(headers || {}).sort();
  }

  function noteSkippedExternal(scope, url, discoveredFrom) {
    if (!scope || scope.skippedExternalSeen.has(url)) {
      return;
    }
    scope.skippedExternalSeen.add(url);
    scope.skippedExternalCount++;
    if (scope.skippedExternalExamples.length < 6) {
      scope.skippedExternalExamples.push({ url, discoveredFrom });
    }
  }

  function isDeepCandidateInScope(url, scope) {
    if (!scope || scope.allowExternalOrigins) {
      return true;
    }
    const origin = httpOriginForURL(url);
    if (!origin || scope.allowedOrigins.has(origin)) {
      return true;
    }
    if (isFamilyOrigin(url, scope)) {
      scope.familyOrigins.add(origin);
      return true;
    }
    if (isExplicitCfAccessTarget(url, scope)) {
      return true;
    }
    return false;
  }

  function decodeReferenceURL(rawURL) {
    return String(rawURL || "")
      .replace(/\\u002[fF]/g, "/")
      .replace(/\\x2[fF]/g, "/")
      .replace(/\\\//g, "/")
      .replace(/&amp;/g, "&")
      .trim();
  }

  function looksLikeAssetReference(rawURL, opts = {}) {
    const url = decodeReferenceURL(rawURL);
    if (!url || url.length > 2048 || /\s|[<>{}]/.test(url) || /\$\{/.test(url)) {
      return false;
    }
    const extPattern = Array.isArray(opts.extraExtensions) && opts.extraExtensions.length
      ? `(?:${collectableExtensions.slice(3, -1)}|${opts.extraExtensions.join("|")})`
      : collectableExtensions;
    if (!new RegExp(`\\.${extPattern}(?:[?#]|$)`, "i").test(url)) {
      return false;
    }
    if (
      opts.allowBare &&
      !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url) &&
      /(?:^|\/)[^/?#]+\.[a-z0-9]+(?:[?#]|$)/i.test(url)
    ) {
      return true;
    }
    return /^(?:https?:|\/\/|\/|\.{1,2}\/|assets\/|static\/|_next\/|build\/|dist\/|chunks\/)/i.test(url);
  }

  function normalizeResourceURL(rawURL, baseURL, pageURL, normalizeOptions = {}) {
    const decoded = decodeReferenceURL(rawURL);
    if (!looksLikeAssetReference(decoded, normalizeOptions)) {
      return "";
    }
    try {
      if (/^\/\//.test(decoded)) {
        const base = new URL(baseURL || pageURL || location.href);
        return `${base.protocol}${decoded}`;
      }
      if (/^assets\//i.test(decoded) && baseURL) {
        const parent = new URL(baseURL);
        const marker = "/assets/";
        const markerIndex = parent.pathname.lastIndexOf(marker);
        if (markerIndex >= 0) {
          parent.pathname = parent.pathname.slice(0, markerIndex + 1) + decoded;
          parent.search = "";
          parent.hash = "";
          return parent.href;
        }
      }
      const base = /^(?:assets|static|_next|build|dist|chunks)\//i.test(decoded)
        ? pageDirectoryURL(pageURL)
        : baseURL || pageDirectoryURL(pageURL);
      const parsed = new URL(decoded, base);
      parsed.hash = "";
      return parsed.href;
    } catch (_error) {
      return "";
    }
  }

  function addCandidate(candidates, seen, candidate, baseURL, pageURL, addOptions = {}) {
    if (!candidate || !candidate.url) {
      return false;
    }
    const url = normalizeResourceURL(candidate.url, baseURL, pageURL, {
      allowBare: !!candidate.allowBare,
      extraExtensions: candidate.extraExtensions,
    });
    if (!url || seen.has(url)) {
      return false;
    }
    if (addOptions.deep && !isDeepCandidateInScope(url, addOptions.scope)) {
      noteSkippedExternal(addOptions.scope, url, candidate.discoveredFrom || baseURL);
      return false;
    }
    if (!/^https?:|^file:|^moz-extension:/i.test(url)) {
      return false;
    }
    const type = inferType(url, candidate.type);
    if (!isCollectableType(type)) {
      return false;
    }
    seen.add(url);
    const stored = { ...candidate, url, type };
    delete stored.allowBare;
    delete stored.extraExtensions;
    candidates.push(stored);
    return stored;
  }

  function decodeJSLiteralBody(body) {
    return decodeReferenceURL(
      String(body || "")
        .replace(/\\n/g, "\n")
        .replace(/\\r/g, "\r")
        .replace(/\\t/g, "\t")
        .replace(/\\(["'`\\])/g, "$1")
    );
  }

  function parseJSStringLiterals(text) {
    const values = [];
    const stringPattern = /"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|`((?:\\.|[^`\\])*)`/g;
    let match;
    while ((match = stringPattern.exec(text || ""))) {
      values.push(decodeJSLiteralBody(match[1] || match[2] || match[3] || ""));
    }
    return values;
  }

  function addRawReference(candidates, seen, rawURL, source, baseURL, pageURL, extra = {}) {
    return addCandidate(candidates, seen, {
      url: rawURL,
      type: inferType(rawURL, ""),
      source,
      discoveredFrom: baseURL,
      ...extra,
    }, baseURL, pageURL);
  }

  function extractViteManifestCandidates(text, baseURL, pageURL, candidates, seen) {
    if (!/^\s*\{/.test(text || "") || !/"(?:file|css|assets|imports|dynamicImports|isEntry|isDynamicEntry)"\s*:/.test(text || "")) {
      return;
    }
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch (_error) {
      return;
    }

    function addManifestValue(rawURL, field, entryKey) {
      addRawReference(candidates, seen, rawURL, "vite-manifest", baseURL, pageURL, {
        manifestField: field,
        manifestEntry: entryKey || "",
      });
    }

    function visitEntry(entry, entryKey) {
      if (!entry || typeof entry !== "object") {
        return;
      }
      for (const field of ["file", "src"]) {
        if (typeof entry[field] === "string") {
          addManifestValue(entry[field], field, entryKey);
        }
      }
      for (const field of ["css", "assets", "imports", "dynamicImports"]) {
        if (!Array.isArray(entry[field])) {
          continue;
        }
        for (const value of entry[field]) {
          if (typeof value === "string") {
            addManifestValue(value, field, entryKey);
          }
        }
      }
    }

    if (parsed && typeof parsed === "object") {
      for (const key of Object.keys(parsed)) {
        visitEntry(parsed[key], key);
      }
    }
  }

  function extractVitePreloadCandidates(text, baseURL, pageURL, candidates, seen) {
    if (!/__vite__mapDeps|modulepreload|vitePreload|rolldown/i.test(text || "")) {
      return;
    }
    const depArrays = [];
    const stringArrayPattern =
      /\[((?:\s*(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`)\s*,?){1,})\]/g;
    let match;
    while ((match = stringArrayPattern.exec(text))) {
      const deps = parseJSStringLiterals(match[1]).filter(value => looksLikeAssetReference(value, { allowBare: true }));
      if (deps.length) {
        depArrays.push(deps);
      }
    }

    const primaryDeps = depArrays.sort((a, b) => b.length - a.length)[0] || [];
    const preloadIndexes = new Set();
    const callPattern = /__vite__mapDeps\(\s*\[([0-9,\s]+)\]/g;
    while ((match = callPattern.exec(text))) {
      for (const value of match[1].split(",")) {
        const index = Number.parseInt(value.trim(), 10);
        if (Number.isFinite(index)) {
          preloadIndexes.add(index);
        }
      }
    }

    if (primaryDeps.length && preloadIndexes.size) {
      for (const index of preloadIndexes) {
        const dep = primaryDeps[index];
        if (dep) {
          addRawReference(candidates, seen, dep, "vite-preload", baseURL, pageURL, {
            viteDepIndex: index,
            allowBare: true,
          });
        }
      }
    }

    for (let index = 0; index < primaryDeps.length; index++) {
      addRawReference(candidates, seen, primaryDeps[index], "vite-map-deps", baseURL, pageURL, {
        viteDepIndex: index,
        allowBare: true,
      });
    }
  }

  function extractJSImportCandidates(text, baseURL, pageURL, candidates, seen) {
    const importPatterns = [
      {
        pattern: new RegExp(String.raw`\bimport\(\s*["'\`]([^"'\`]+\.${collectableExtensions}(?:[?#][^"'\`]*)?)["'\`]`, "gi"),
        source: "js-dynamic-import",
      },
      {
        pattern: new RegExp(String.raw`\bfrom\s*["'\`]([^"'\`]+\.${collectableExtensions}(?:[?#][^"'\`]*)?)["'\`]`, "gi"),
        source: "js-static-import",
      },
      {
        pattern: new RegExp(String.raw`\bimport\s*["'\`]([^"'\`]+\.${collectableExtensions}(?:[?#][^"'\`]*)?)["'\`]`, "gi"),
        source: "js-side-effect-import",
      },
    ];
    for (const entry of importPatterns) {
      let match;
      while ((match = entry.pattern.exec(text))) {
        addRawReference(candidates, seen, match[1], entry.source, baseURL, pageURL, {
          allowBare: true,
        });
      }
    }
  }

  function extractImportMetaURLCandidates(text, baseURL, pageURL, candidates, seen) {
    const pattern = new RegExp(
      String.raw`new\s+URL\(\s*["'\`]([^"'\`]+\.${collectableExtensions}(?:[?#][^"'\`]*)?)["'\`]\s*,\s*import\.meta\.url\s*\)`,
      "gi"
    );
    let match;
    while ((match = pattern.exec(text))) {
      addRawReference(candidates, seen, match[1], "js-import-meta-url", baseURL, pageURL, {
        allowBare: true,
      });
    }
  }

  function joinAssetPath(path, file) {
    const base = String(path || "");
    if (!base) {
      return file;
    }
    return (base.endsWith("/") ? base : `${base}/`) + file;
  }

  function extractThreeLoaderCandidates(text, baseURL, pageURL, candidates, seen) {
    const source = String(text || "");
    const dracoFiles = ["draco_decoder.js", "draco_wasm_wrapper.js", "draco_decoder.wasm"];
    const basisFiles = ["basis_transcoder.js", "basis_transcoder.wasm"];

    function addFiles(paths, files, sourceLabel) {
      for (const path of paths) {
        for (const file of files) {
          addRawReference(candidates, seen, joinAssetPath(path, file), sourceLabel, baseURL, pageURL, {
            allowBare: true,
          });
        }
      }
    }

    const decoderPaths = [];
    const decoderPathPattern = /\.setDecoderPath\(\s*["'`]([^"'`]+)["'`]\s*\)|decoderPath\s*[:=]\s*["'`]([^"'`]+)["'`]/g;
    let match;
    while ((match = decoderPathPattern.exec(source))) {
      decoderPaths.push(match[1] || match[2] || "");
    }
    if (decoderPaths.length) {
      addFiles(decoderPaths, dracoFiles, "three-draco-loader");
    }

    const transcoderPaths = [];
    const transcoderPathPattern = /\.setTranscoderPath\(\s*["'`]([^"'`]+)["'`]\s*\)|transcoderPath\s*[:=]\s*["'`]([^"'`]+)["'`]/g;
    while ((match = transcoderPathPattern.exec(source))) {
      transcoderPaths.push(match[1] || match[2] || "");
    }
    if (transcoderPaths.length) {
      addFiles(transcoderPaths, basisFiles, "three-ktx2-loader");
    }

    const literals = parseJSStringLiterals(source);
    for (const literal of literals) {
      if (dracoFiles.includes(literal.split("/").pop())) {
        addRawReference(candidates, seen, literal, "three-draco-loader", baseURL, pageURL, {
          allowBare: true,
        });
      } else if (basisFiles.includes(literal.split("/").pop())) {
        addRawReference(candidates, seen, literal, "three-ktx2-loader", baseURL, pageURL, {
          allowBare: true,
        });
      }
    }
  }

  // Emscripten / wasm-bindgen side files. These extensions (.data/.mem/.wasm.map)
  // are not in the global collectable set to avoid false positives, so we add the
  // specific string literals found in glue code as candidates directly.
  function extractWasmSideFileCandidates(text, baseURL, pageURL, candidates, seen) {
    const source = String(text || "");
    if (!/\.(?:data|mem|wasm\.map)\b|wasm_binary_file|locateFile|wbg|wasm-bindgen/i.test(source)) {
      return;
    }
    // .data preload packages, .mem memory init files, and .wasm.map source maps.
    const sideFilePattern = /["'`]([^"'`\s<>]{1,2048}\.(?:data|mem|wasm\.map)(?:[?#][^"'`\s<>)]*)?)["'`]/gi;
    let match;
    while ((match = sideFilePattern.exec(source))) {
      const url = match[1];
      // Skip obvious non-asset matches (e.g. mime types, "form-data").
      if (/[\s:;]/.test(url) || /^data$/i.test(url)) {
        continue;
      }
      const type = /\.wasm\.map(?:[?#]|$)/i.test(url) ? "source-map" : "wasm";
      addCandidate(candidates, seen, {
        url,
        type,
        source: "wasm-side-file",
        discoveredFrom: baseURL,
        allowBare: true,
        extraExtensions: ["data", "mem"],
      }, baseURL, pageURL);
    }
  }

  function extractReferencedResourceCandidates(text, baseURL, pageURL) {
    if (!text) {
      return [];
    }
    const candidates = [];
    const seen = new Set();
    const quotedPattern = new RegExp(
      String.raw`["'\`]([^"'\`\s<>]{1,2048}\.${collectableExtensions}(?:[?#][^"'\`\s<>)]*)?)["'\`]`,
      "gi"
    );
    const cssURLPattern = new RegExp(
      String.raw`url\(\s*["']?([^"')\s]{1,2048}\.${collectableExtensions}(?:[?#][^"')\s]*)?)["']?\s*\)`,
      "gi"
    );
    const htmlAttrPattern = new RegExp(
      String.raw`(?:src|href)=["']([^"']{1,2048}\.${collectableExtensions}(?:[?#][^"']*)?)["']`,
      "gi"
    );

    function add(rawURL, source) {
      addRawReference(candidates, seen, rawURL, source, baseURL, pageURL);
    }

    extractViteManifestCandidates(text, baseURL, pageURL, candidates, seen);
    extractVitePreloadCandidates(text, baseURL, pageURL, candidates, seen);
    extractJSImportCandidates(text, baseURL, pageURL, candidates, seen);
    extractImportMetaURLCandidates(text, baseURL, pageURL, candidates, seen);
    extractThreeLoaderCandidates(text, baseURL, pageURL, candidates, seen);
    extractWasmSideFileCandidates(text, baseURL, pageURL, candidates, seen);

    for (const pattern of [quotedPattern, cssURLPattern, htmlAttrPattern]) {
      let match;
      while ((match = pattern.exec(text))) {
        add(match[1], "bundle-reference");
      }
    }

    return candidates;
  }

  // Web App Manifest (manifest.webmanifest / manifest.json): mine icon,
  // screenshot, and shortcut-icon asset URLs.
  function extractWebManifestCandidates(text, baseURL, pageURL) {
    const source = String(text || "");
    if (!/^\s*\{/.test(source) || !/"(?:icons|screenshots|shortcuts|start_url)"\s*:/.test(source)) {
      return [];
    }
    let parsed;
    try {
      parsed = JSON.parse(source);
    } catch (_error) {
      return [];
    }
    const candidates = [];
    const seen = new Set();
    function addEntries(list) {
      if (!Array.isArray(list)) {
        return;
      }
      for (const entry of list) {
        if (entry && typeof entry.src === "string") {
          addRawReference(candidates, seen, entry.src, "web-manifest-asset", baseURL, pageURL, {
            allowBare: true,
          });
        }
        if (entry && Array.isArray(entry.icons)) {
          addEntries(entry.icons);
        }
      }
    }
    addEntries(parsed.icons);
    addEntries(parsed.screenshots);
    addEntries(parsed.shortcuts);
    return candidates;
  }

  function looksLikeServiceWorkerManifest(text, url) {
    if (/service-worker|sw\.js/i.test(url || "")) {
      return true;
    }
    return /precacheAndRoute|__WB_MANIFEST|workbox|["']revision["']\s*:/.test(text || "");
  }

  function extractServiceWorkerPrecacheCandidates(text, baseURL, pageURL) {
    if (!text || !looksLikeServiceWorkerManifest(text, baseURL)) {
      return [];
    }
    const candidates = [];
    const seen = new Set();
    const objectPattern = /\{[^{}]{0,4096}\}/g;
    const urlFieldPattern = new RegExp(
      String.raw`(?:["']url["']|url)\s*:\s*["']([^"']{1,2048}\.${collectableExtensions}(?:[?#][^"']*)?)["']`,
      "gi"
    );
    const revisionPattern = /(?:["']revision["']|revision)\s*:\s*["']([^"']*)["']/i;
    const integrityPattern = /(?:["']integrity["']|integrity)\s*:\s*["']([^"']*)["']/i;

    function add(rawURL, meta = {}) {
      addCandidate(candidates, seen, {
        url: rawURL,
        type: inferType(rawURL, ""),
        source: "service-worker-precache",
        discoveredFrom: baseURL,
        precacheRevision: meta.revision || null,
        precacheIntegrity: meta.integrity || null,
      }, baseURL, pageURL);
    }

    let match;
    while ((match = objectPattern.exec(text))) {
      const chunk = match[0];
      const urlMatch = new RegExp(
        String.raw`(?:["']url["']|url)\s*:\s*["']([^"']{1,2048}\.${collectableExtensions}(?:[?#][^"']*)?)["']`,
        "i"
      ).exec(chunk);
      if (!urlMatch) {
        continue;
      }
      const revisionMatch = revisionPattern.exec(chunk);
      const integrityMatch = integrityPattern.exec(chunk);
      add(urlMatch[1], {
        revision: revisionMatch ? revisionMatch[1] : null,
        integrity: integrityMatch ? integrityMatch[1] : null,
      });
    }
    while ((match = urlFieldPattern.exec(text))) {
      add(match[1]);
    }
    for (const literal of parseJSStringLiterals(text)) {
      if (looksLikeAssetReference(literal, { allowBare: true })) {
        add(literal);
      }
    }
    return candidates;
  }

  // Reconstruct webpack/Turbopack code-split chunk URLs. Bundlers build chunk
  // paths at runtime as `prefix + chunkId + map[chunkId] + ".js"`, so the chunk
  // filenames never appear as whole string literals. We recover them by pairing
  // the chunk-id -> hash map object with the surrounding path template.
  function extractWebpackChunkMapCandidates(text, baseURL, pageURL, candidates, seen) {
    const source = String(text || "");
    if (!/__webpack_require__|webpackChunk|\.u\s*=|\bchunkId\b|turbopack/i.test(source)) {
      return;
    }
    // chunk-id -> hash maps: {123:"abc",456:"def"} or {"app":"a1b2"}
    const mapPattern = /\{((?:\s*(?:[0-9]+|"[^"]+"|'[^']+')\s*:\s*"[0-9a-f]{6,}"\s*,?){2,})\}/gi;
    // Path template prefixes/suffixes around the map, e.g. "static/chunks/" ... ".js"
    const prefixPattern = /["'`]((?:[\w./-]*?(?:chunks|static|assets|_next)[\w./-]*?\/)?)["'`]/gi;
    const prefixes = new Set(["", "static/chunks/", "chunks/", "_next/static/chunks/"]);
    let pmatch;
    while ((pmatch = prefixPattern.exec(source))) {
      if (pmatch[1]) {
        prefixes.add(pmatch[1]);
      }
    }
    let match;
    while ((match = mapPattern.exec(source))) {
      const entries = match[1].match(/(?:[0-9]+|"[^"]+"|'[^']+')\s*:\s*"[0-9a-f]{6,}"/gi) || [];
      for (const entry of entries) {
        const kv = /^(?:([0-9]+)|"([^"]+)"|'([^']+)')\s*:\s*"([0-9a-f]{6,})"$/i.exec(entry);
        if (!kv) {
          continue;
        }
        const id = kv[1] || kv[2] || kv[3];
        const hash = kv[4];
        for (const prefix of prefixes) {
          // Common webpack/Turbopack chunk filename shapes.
          for (const file of [`${prefix}${id}.${hash}.js`, `${prefix}${id}-${hash}.js`, `${prefix}${hash}.js`]) {
            addRawReference(candidates, seen, file, "webpack-chunk-map", baseURL, pageURL, {
              allowBare: true,
              chunkId: id,
              chunkHash: hash,
            });
          }
        }
      }
    }
  }

  // Mine inline script bodies (including RSC/Flight `self.__next_f.push(...)`
  // rows and Module Federation remoteEntry references) for asset URLs and chunk
  // maps. These never appear in document.scripts[].src yet routinely embed the
  // real chunk paths a crawl would otherwise miss.
  function extractInlineScriptCandidates(snapshot, candidates, seen, pageURL) {
    const texts = [];
    for (const script of snapshot.scripts || []) {
      if (!script.src && script.text) {
        texts.push(script.text);
      }
    }
    for (const dataScript of snapshot.dataScripts || []) {
      if (dataScript.text) {
        texts.push(dataScript.text);
      }
    }
    if (snapshot.hydrationGlobals && typeof snapshot.hydrationGlobals.__NEXT_F !== "undefined") {
      try {
        texts.push(JSON.stringify(snapshot.hydrationGlobals.__NEXT_F));
      } catch (_error) {
        // Ignore non-serializable flight payloads.
      }
    }
    for (const text of texts) {
      for (const candidate of extractReferencedResourceCandidates(text, pageURL, pageURL)) {
        addCandidate(candidates, seen, candidate, null, pageURL);
      }
      extractWebpackChunkMapCandidates(text, pageURL, pageURL, candidates, seen);
    }
  }

  function discoverGeneratedResourceCandidates(snapshot) {
    const candidates = [];
    const seen = new Set();
    const pageURL = snapshot.href || location.href;

    for (const script of snapshot.scripts || []) {
      addCandidate(candidates, seen, {
        url: script.src,
        type: "script",
        source: "document.scripts",
      }, null, pageURL);
    }

    for (const link of snapshot.links || []) {
      const rel = String(link.rel || "");
      const type = /stylesheet/i.test(rel) || link.as === "style" ? "stylesheet" : link.as;
      addCandidate(candidates, seen, {
        url: link.href,
        type,
        source: "document.links",
      }, null, pageURL);
    }

    for (const sheet of snapshot.styleSheets || []) {
      addCandidate(candidates, seen, {
        url: sheet.href,
        type: "stylesheet",
        source: "document.styleSheets",
      }, null, pageURL);
    }

    for (const entry of snapshot.performanceResources || []) {
      const initiatorType = entry.initiatorType === "css" ? "stylesheet" : entry.initiatorType;
      addCandidate(candidates, seen, {
        url: entry.name,
        type: initiatorType,
        source: "performance.resource",
      }, null, pageURL);
    }

    // Cross-origin iframes can't be walked, but their document URL is still a
    // fetchable HTML asset; same-origin frame assets were already merged into
    // the script/link/style collections above (tagged with framePath).
    for (const frame of snapshot.frames || []) {
      if (!frame.accessible && frame.src) {
        addCandidate(candidates, seen, {
          url: frame.src,
          type: "html",
          source: "iframe",
        }, null, pageURL);
      }
    }

    const pageOrigin = httpOriginForURL(pageURL);
    function addImportMapEntry(rawURL, source) {
      const resolved = normalizeResourceURL(rawURL, null, pageURL);
      // importmap values may point to third-party CDNs; keep same-origin only
      // (bare specifiers / relative paths resolve to the page origin and pass).
      const origin = httpOriginForURL(resolved || rawURL);
      if (origin && pageOrigin && origin !== pageOrigin) {
        return;
      }
      addCandidate(candidates, seen, {
        url: rawURL,
        type: "script",
        source,
      }, null, pageURL);
    }
    for (const map of snapshot.importMaps || []) {
      const imports = (map && map.imports) || {};
      for (const specifier of Object.keys(imports)) {
        addImportMapEntry(imports[specifier], "importmap");
      }
      const scopes = (map && map.scopes) || {};
      for (const scopeKey of Object.keys(scopes)) {
        const scopeImports = scopes[scopeKey] || {};
        for (const specifier of Object.keys(scopeImports)) {
          addImportMapEntry(scopeImports[specifier], "importmap-scope");
        }
      }
    }

    const hookEventKindToType = {
      worker: "script",
      "service-worker": "script",
      wasm: "wasm",
    };
    const hookEvents = snapshot.runtimeHook ? snapshot.runtimeHook.events || [] : [];
    for (const event of hookEvents) {
      const payload = event.payload || {};
      for (const ref of payload.references || []) {
        addCandidate(candidates, seen, {
          url: ref.url,
          type: ref.type || inferType(ref.url, ""),
          source: ref.source || (event.kind === "worker" ? "runtime-blob-worker-reference" : "runtime-blob-reference"),
          discoveredFrom: payload.url || event.href || pageURL,
          allowBare: true,
        }, payload.url && !/^blob:|^data:/i.test(payload.url) ? payload.url : event.href || pageURL, pageURL);
      }
      const mappedType = hookEventKindToType[event.kind];
      const eventURL = payload.src || payload.href || payload.name || payload.url;
      if (!eventURL) {
        continue;
      }
      addCandidate(candidates, seen, {
        url: eventURL,
        type: payload.rel && /stylesheet/i.test(payload.rel)
          ? "stylesheet"
          : payload.as || mappedType || event.kind,
        source: event.kind === "worker" || event.kind === "service-worker"
          ? "runtime-worker"
          : event.kind === "wasm"
            ? "runtime-wasm"
            : "runtime-hook",
      }, null, pageURL);
    }

    extractInlineScriptCandidates(snapshot, candidates, seen, pageURL);

    // srcset / imagesrcset: comma-separated "<url> <descriptor>" pairs.
    function addSrcset(value, source) {
      for (const part of String(value || "").split(",")) {
        const url = part.trim().split(/\s+/)[0];
        if (url) {
          addRawReference(candidates, seen, url, source, null, pageURL, { allowBare: true });
        }
      }
    }
    for (const entry of snapshot.srcsets || []) {
      addSrcset(entry.srcset, "img-srcset");
      if (entry.src) {
        addRawReference(candidates, seen, entry.src, "img-src", null, pageURL, { allowBare: true });
      }
    }
    for (const link of snapshot.links || []) {
      if (link.imagesrcset) {
        addSrcset(link.imagesrcset, "link-imagesrcset");
      }
    }

    return candidates;
  }

  function credentialsForFetch(url, scope) {
    const origin = httpOriginForURL(url);
    if (!origin || !scope) {
      return "omit";
    }
    if (
      scope.useSessionCredentials &&
      (origin === scope.pageOrigin ||
        scope.allowedOrigins.has(origin) ||
        isFamilyOrigin(url, scope) ||
        isExplicitCfAccessTarget(url, scope))
    ) {
      return "include";
    }
    return "omit";
  }

  function bodyPrefix(content) {
    if (typeof content !== "string") {
      return "";
    }
    return content.replace(/\s+/g, " ").trim().slice(0, 500);
  }

  // A SPA commonly answers unmatched asset paths with 200 + index.html. Detect that
  // so we don't save the HTML shell under a .js/.css name (and don't deep-crawl it).
  function isHtmlSoftError(requestedType, contentType, content) {
    if (requestedType === "html") {
      return false;
    }
    if (/^\s*text\/html\b/i.test(contentType || "")) {
      return true;
    }
    if (typeof content === "string") {
      const head = content.slice(0, 512).toLowerCase();
      if (/^\s*<!doctype html/.test(head) || /^\s*<html[\s>]/.test(head)) {
        return true;
      }
    }
    return false;
  }

  function htmlSoftErrorPhase(content, finalURL) {
    const haystack = `${finalURL || ""}\n${String(content || "").slice(0, 4000)}`.toLowerCase();
    if (/cloudflare access|cf-access-domain|\/cdn-cgi\/access\/login|sign in .*cloudflare access/.test(haystack)) {
      return "auth-required";
    }
    return "html-soft-error";
  }

  // Build a fetched-resource record from a passively captured network response,
  // so we serve exactly what the page received instead of re-fetching.
  function resourceFromNetworkCapture(candidate, capture) {
    const contentType = capture.contentType || "";
    const requestedType = candidate.type || inferType(candidate.url, "");
    const resourceType = typeFromContentType(contentType) || requestedType;
    const content = isTextResource(resourceType, contentType)
      ? textDecoder.decode(capture.bytes)
      : capture.bytes;
    const softError = requestedType !== "html"
      && resourceType !== "html"
      && isHtmlSoftError(requestedType, contentType, content);
    return {
      content,
      contentType,
      sourceMapHeader: capture.sourceMapHeader || "",
      resourceType,
      softError,
      softErrorPhase: softError ? htmlSoftErrorPhase(content, candidate.url) : "",
      status: capture.status == null ? 200 : capture.status,
      statusText: capture.statusText || "",
      finalURL: candidate.url,
      credentialMode: "network-capture",
      requestHeaderNames: [],
      bodyPrefix: bodyPrefix(content),
      fromNetworkCapture: true,
    };
  }

  async function fetchGeneratedResource(candidate, scope) {
    // Prefer the actual response the page received, when captured.
    if (scope && scope.preferNetworkCapture) {
      const capture = lookupNetworkCapture(candidate.url);
      if (capture && (capture.status == null || (capture.status >= 200 && capture.status < 300))) {
        return resourceFromNetworkCapture(candidate, capture);
      }
    }
    const credentialMode = credentialsForFetch(candidate.url, scope);
    const headers = requestHeadersForFetch(candidate.url, scope);
    const response = await fetch(candidate.url, {
      credentials: credentialMode,
      cache: "no-store",
      headers,
    });
    const contentType = response.headers.get("content-type") || "";
    const sourceMapHeader = response.headers.get("sourcemap")
      || response.headers.get("x-sourcemap")
      || "";
    const requestedType = candidate.type || inferType(candidate.url, "");
    const resourceType = typeFromContentType(contentType) || requestedType;
    const bytes = new Uint8Array(await response.arrayBuffer());
    const content = isTextResource(resourceType, contentType)
      ? textDecoder.decode(bytes)
      : bytes;
    if (!response.ok) {
      const error = new Error(`HTTP ${response.status}`);
      error.status = response.status;
      error.statusText = response.statusText || "";
      error.contentType = contentType;
      error.finalURL = response.url || candidate.url;
      error.bodyPrefix = bodyPrefix(content);
      error.credentialMode = credentialMode;
      error.requestHeaderNames = requestHeaderNames(headers);
      throw error;
    }
    // Reject HTML soft-404s unless an HTML resource was actually requested.
    const softError = requestedType !== "html"
      && resourceType !== "html"
      && isHtmlSoftError(requestedType, contentType, content);
    return {
      content,
      contentType,
      sourceMapHeader,
      resourceType,
      softError,
      softErrorPhase: softError ? htmlSoftErrorPhase(content, response.url || candidate.url) : "",
      status: response.status,
      statusText: response.statusText || "",
      finalURL: response.url || candidate.url,
      credentialMode,
      requestHeaderNames: requestHeaderNames(headers),
      bodyPrefix: bodyPrefix(content),
    };
  }

  async function collectGeneratedResourcesFromPageSnapshot(snapshot, scanOptions) {
    const resources = discoverGeneratedResourceCandidates(snapshot);
    const seen = new Set(resources.map(resource => resource.url));
    const pageURL = snapshot.href || location.href;
    log(`Discovered ${resources.length} generated/resource URLs from page runtime data.`);
    const deepCrawlScope = createDeepCrawlScope(pageURL, resources, scanOptions);
    if (deepCrawlScope.useSessionCredentials) {
      log("Using browser session credentials for in-scope page, loaded-origin, and first-party-family fetches.");
    }
    if (deepCrawlScope.allowFirstPartyFamily && deepCrawlScope.familyDomains.length) {
      log(`First-party family domains: ${deepCrawlScope.familyDomains.join(", ")}`);
    }
    if (deepCrawlScope.cfAccessClientId && deepCrawlScope.cfAccessClientSecret) {
      const cfScope = deepCrawlScope.cfAccessDomains.length
        ? deepCrawlScope.cfAccessDomains.join(", ")
        : "in-scope page, loaded-origin, and first-party-family origins";
      log(`Using CF Access service-token headers for: ${cfScope}.`);
    }

    let fetched = 0;
    let deepDiscovered = 0;
    let serviceWorkerManifestDiscovered = 0;
    let softErrors = 0;
    const fetchFailures = [];
    const processResource = async (resource) => {
      let fetchedResource;
      try {
        fetchedResource = await fetchGeneratedResource(resource, deepCrawlScope);
      } catch (error) {
        const record = addResourceError({
          phase: "fetch",
          url: resource.url,
          resourceType: resource.type || null,
          source: resource.source || null,
          message: error.message,
          ...errorDetails(error),
        });
        if (resource.source === "service-worker-precache") {
          upsertServiceWorkerAsset(resource, {
            present: false,
            status: "fetch-failed",
            errorPhase: record.phase,
            errorMessage: record.message,
            httpStatus: record.status,
            contentType: record.contentType,
            finalUrl: record.finalURL,
            credentialMode: record.credentialMode,
            requestHeaderNames: record.requestHeaderNames || [],
          });
        }
        fetchFailures.push(record);
        if (fetchFailures.length <= 8) {
          log(`Fetch failed: ${resource.url} (${error.message})`);
        }
        return;
      }

      // Drop SPA HTML soft-404s: don't save them and don't deep-crawl their markup.
      if (fetchedResource.softError) {
        softErrors++;
        addResourceError({
          phase: fetchedResource.softErrorPhase,
          url: resource.url,
          resourceType: resource.type || null,
          source: resource.source || null,
          message: fetchedResource.softErrorPhase === "auth-required"
            ? "Received an authentication HTML page instead of the requested asset"
            : "Received HTML instead of the requested asset",
          status: fetchedResource.status,
          statusText: fetchedResource.statusText,
          contentType: fetchedResource.contentType,
          finalURL: fetchedResource.finalURL,
          bodyPrefix: fetchedResource.bodyPrefix,
          credentialMode: fetchedResource.credentialMode,
          requestHeaderNames: fetchedResource.requestHeaderNames,
        });
        if (resource.source === "service-worker-precache") {
          upsertServiceWorkerAsset(resource, {
            present: false,
            status: fetchedResource.softErrorPhase,
            errorPhase: fetchedResource.softErrorPhase,
            errorMessage: fetchedResource.softErrorPhase === "auth-required"
              ? "Received an authentication HTML page instead of the requested asset"
              : "Received HTML instead of the requested asset",
            httpStatus: fetchedResource.status,
            contentType: fetchedResource.contentType,
            finalUrl: fetchedResource.finalURL,
            credentialMode: fetchedResource.credentialMode,
            requestHeaderNames: fetchedResource.requestHeaderNames || [],
          });
        }
        if (softErrors <= 5) {
          const label = fetchedResource.softErrorPhase === "auth-required"
            ? "HTML auth page"
            : "HTML soft-404";
          log(`Skipped ${label} (expected ${resource.type || "asset"}): ${resource.url}`);
        }
        return;
      }
      fetched++;

      const resourceType = fetchedResource.resourceType;
      const generatedItem = {
        bucket: "generated",
        resourceType,
        url: resource.url,
        content: fetchedResource.content,
        encoding: "plain",
        contentType: fetchedResource.contentType,
        sourceMapHeader: fetchedResource.sourceMapHeader || "",
        source: resource.source,
        discoveredFrom: resource.discoveredFrom || null,
        status: fetchedResource.status,
        statusText: fetchedResource.statusText,
        finalUrl: fetchedResource.finalURL,
        credentialMode: fetchedResource.credentialMode,
        requestHeaderNames: fetchedResource.requestHeaderNames,
        manifestField: resource.manifestField || null,
        manifestEntry: resource.manifestEntry || null,
        viteDepIndex: resource.viteDepIndex == null ? null : resource.viteDepIndex,
        precacheRevision: resource.precacheRevision || null,
        precacheIntegrity: resource.precacheIntegrity || null,
      };
      const storedGeneratedItem = addItem(generatedItem);
      if (resource.source === "service-worker-precache") {
        upsertServiceWorkerAsset(resource, {
          present: true,
          status: "fetched",
          httpStatus: fetchedResource.status,
          contentType: fetchedResource.contentType,
          finalUrl: fetchedResource.finalURL,
          credentialMode: fetchedResource.credentialMode,
          requestHeaderNames: fetchedResource.requestHeaderNames || [],
          errorPhase: null,
          errorMessage: null,
        });
      }

      const currentDepth = resource.depth || 0;
      if (
        typeof fetchedResource.content === "string" &&
        currentDepth < deepCrawlScope.maxDepth &&
        deepCrawlScope.deepAdded < deepCrawlScope.maxCandidates
      ) {
        const chunkMapCandidates = [];
        const chunkMapSeen = new Set();
        extractWebpackChunkMapCandidates(
          fetchedResource.content,
          resource.url,
          pageURL,
          chunkMapCandidates,
          chunkMapSeen
        );
        const nested = [
          ...extractServiceWorkerPrecacheCandidates(fetchedResource.content, resource.url, pageURL),
          ...extractWebManifestCandidates(fetchedResource.content, resource.url, pageURL),
          ...extractReferencedResourceCandidates(fetchedResource.content, resource.url, pageURL),
          ...chunkMapCandidates,
        ];
        for (const candidate of nested) {
          if (deepCrawlScope.deepAdded >= deepCrawlScope.maxCandidates) {
            if (!deepCrawlScope.capNotified) {
              deepCrawlScope.capNotified = true;
              log(`Deep crawl candidate cap (${deepCrawlScope.maxCandidates}) reached; stopping discovery.`);
            }
            break;
          }
          candidate.depth = currentDepth + 1;
          const added = addCandidate(resources, seen, candidate, resource.url, pageURL, {
            deep: true,
            scope: deepCrawlScope,
          });
          if (added) {
            deepDiscovered++;
            if (added.source === "service-worker-precache") {
              serviceWorkerManifestDiscovered++;
              upsertServiceWorkerAsset(added, { status: "queued" });
            }
            deepCrawlScope.deepAdded++;
          }
        }
      }

      if (
        scanOptions.resolveSourceMaps &&
        typeof storedGeneratedItem.content === "string" &&
        (resourceType === "script" || resourceType === "stylesheet")
      ) {
        const resolution = await MindemonSourceMaps.resolveForText(
          storedGeneratedItem,
          {
            fetchMissingSources: scanOptions.fetchMissingSources,
            allowExternalOrigins: deepCrawlScope.allowExternalOrigins,
            pageOrigin: deepCrawlScope.pageOrigin,
            pageURL,
            allowedOrigins: deepCrawlScope.allowedOrigins,
            allowFirstPartyFamily: deepCrawlScope.allowFirstPartyFamily,
            familyDomains: deepCrawlScope.familyDomains,
            useSessionCredentials: deepCrawlScope.useSessionCredentials,
            cloudflareAccess: {
              clientId: deepCrawlScope.cfAccessClientId,
              clientSecret: deepCrawlScope.cfAccessClientSecret,
              domains: deepCrawlScope.cfAccessDomains,
            },
          }
        );
        for (const error of resolution.errors) {
          const record = addResourceError({
            ...error,
            generatedUrl: storedGeneratedItem.url,
            resourceType,
            source: "source-map",
          });
          storedGeneratedItem.errors = storedGeneratedItem.errors || [];
          storedGeneratedItem.errors.push(record);
          // A guessed `.map` (from a debugId) that 404s is expected, not a failure.
          if (error.phase === "debugid-guess-miss") {
            continue;
          }
          log(`Source map ${error.phase} failed: ${error.url || storedGeneratedItem.url} (${error.message})`);
        }
        if (resolution.debugId && resolution.mapSource === "debugId-guess" && resolution.mapText) {
          log(`Recovered source map via debugId guess (${resolution.debugId}): ${resolution.mapURL}`);
        }
        if (resolution.mapText) {
          addItem({
            bucket: "source-map",
            resourceType: "source-map",
            url: resolution.mapURL,
            generatedUrl: storedGeneratedItem.url,
            content: resolution.mapText,
            source: "sourceMappingURL",
          });
        }
        for (const original of resolution.originals) {
          if (!original.content) {
            continue;
          }
          addItem({
            bucket: "original",
            resourceType,
            url: original.url,
            generatedUrl: original.generatedURL,
            mapUrl: original.mapURL,
            content: original.content,
            contentSource: original.contentSource,
            source: "source-map",
          });
        }
      }
    };

    // Bounded-concurrency pool. The `resources` array grows while we work (deep
    // crawl appends newly-discovered candidates), so the pool re-checks the queue
    // length whenever a task finishes and only resolves once the cursor has
    // caught up and no task is in flight.
    const FETCH_CONCURRENCY = 6;
    await new Promise((resolve, reject) => {
      let cursor = 0;
      let active = 0;
      let settled = false;
      const pump = () => {
        if (settled) {
          return;
        }
        if (cursor >= resources.length && active === 0) {
          settled = true;
          resolve();
          return;
        }
        while (active < FETCH_CONCURRENCY && cursor < resources.length) {
          const resource = resources[cursor++];
          active++;
          processResource(resource).then(
            () => {
              active--;
              pump();
            },
            (error) => {
              settled = true;
              reject(error);
            }
          );
        }
      };
      pump();
    });
    if (fetchFailures.length > 8) {
      log(`Suppressed ${fetchFailures.length - 8} additional fetch failure log(s).`);
    }
    if (softErrors) {
      log(`Skipped ${softErrors} HTML soft-404 response(s) served for asset URLs.`);
    }
    if (serviceWorkerManifestDiscovered) {
      log(`Service worker precache manifest added ${serviceWorkerManifestDiscovered} asset URL(s).`);
    }
    if (deepCrawlScope.allowFirstPartyFamily && deepCrawlScope.familyOrigins.size) {
      log(
        `First-party family mining reached ${deepCrawlScope.familyOrigins.size} related origin(s): ` +
          Array.from(deepCrawlScope.familyOrigins).join(", ")
      );
    }
    if (deepCrawlScope.skippedExternalCount) {
      const examples = deepCrawlScope.skippedExternalExamples
        .map(example => example.url)
        .join(", ");
      log(
        `Skipped ${deepCrawlScope.skippedExternalCount} out-of-scope deep URL(s). ` +
          `Enable "First-party family" or "Deep crawl external origins" to follow them. Examples: ${examples}`
      );
    }
    log(`Fetched ${fetched} generated/resources; deep crawl added ${deepDiscovered} referenced URLs.`);
  }

  function addPrettyFallbacks() {
    const originalsByGenerated = new Set(
      state.items
        .filter(item => item.bucket === "original" && item.generatedUrl)
        .map(item => item.generatedUrl)
    );
    const generated = state.items.filter(
      item =>
        item.bucket === "generated" &&
        typeof item.content === "string" &&
        item.minified &&
        (item.resourceType === "script" || item.resourceType === "stylesheet") &&
        !originalsByGenerated.has(item.url)
    );

    for (const item of generated) {
      addItem({
        bucket: "pretty",
        resourceType: item.resourceType,
        url: item.url + ".pretty",
        generatedUrl: item.url,
        content: prettyPrint(item.content, item.resourceType),
        source: "pretty-fallback",
      });
    }
  }

  function captureMatch(patterns, text) {
    for (const pattern of patterns) {
      pattern.lastIndex = 0;
      const match = pattern.exec(text);
      if (match) {
        return {
          value: match[1] || match[2] || match[3] || "",
          index: match.index,
          matched: match[0].slice(0, 160),
        };
      }
    }
    return null;
  }

  function detectFrameworkInventory() {
    const detectors = [
      {
        name: "React",
        markers: [/ReactCurrentDispatcher|react\.production|react\.development|jsxDEV|react\.memo/],
        versions: [/React\.version\s*=\s*["'`]([^"'`]+)["'`]/, /\bversion\s*[:=]\s*["'`]([0-9]+\.[0-9]+\.[0-9]+(?:-[^"'`]+)?)["'`]/],
      },
      {
        name: "React DOM",
        markers: [/rendererPackageName\s*[:=]\s*["'`]react-dom["'`]|react-dom|reconcilerVersion/],
        versions: [/reconcilerVersion\s*[:=]\s*["'`]([^"'`]+)["'`]/, /\bversion\s*[:=]\s*["'`]([0-9]+\.[0-9]+\.[0-9]+(?:-[^"'`]+)?)["'`]/],
      },
      {
        name: "Workbox",
        markers: [/workbox/],
        versions: [/workbox:[\w-]+:([0-9]+\.[0-9]+\.[0-9]+)/],
      },
      {
        name: "Three.js",
        markers: [/three\.js|REVISION|DRACOLoader|KTX2Loader/],
        versions: [/three\.js r([0-9]+)/, /REVISION\s*[:=]\s*["'`]([0-9]+)["'`]/],
        versionPrefix: "r",
      },
      {
        name: "Mapbox GL JS",
        markers: [/mapbox-gl|mapboxgl|Mapbox/],
        versions: [/mapbox-gl-js\/v?([0-9]+\.[0-9]+\.[0-9]+)/, /\bversion\s*[:=]\s*["'`]([0-9]+\.[0-9]+\.[0-9]+)["'`]/],
      },
      {
        name: "Lexical",
        markers: [/\blexical\b|LexicalEditor|LexicalComposer/i],
        versions: [/([0-9]+\.[0-9]+\.[0-9]+)\+prod\.esm/],
      },
      {
        name: "Fuse.js",
        markers: [/Fuse\.js|\bFuse\b/],
        versions: [/Fuse\.js v?([0-9]+\.[0-9]+\.[0-9]+)/, /\bversion\s*[:=]\s*["'`]([0-9]+\.[0-9]+\.[0-9]+)["'`]/],
      },
      {
        name: "React Stripe.js",
        markers: [/react-stripe|@stripe\/stripe-js|ElementsConsumer|useStripe/],
        versions: [/react-stripe(?:\.esm)?[^0-9]{0,80}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Preact",
        markers: [/\bpreact\b|options\.__b|__H/],
        versions: [/preact[^0-9]{0,80}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "TanStack Query",
        markers: [/QueryClient|useSuspenseQuery|infiniteQueryOptions|gcTime|mutationKey/],
        versions: [],
        inferredVersion: "v5-era APIs",
      },
      {
        name: "Zustand",
        markers: [/\bzustand\b|useStoreApi|createStoreImpl/],
        versions: [],
      },
      {
        name: "Motion",
        markers: [/framer-motion|motion\/react|MotionConfigContext|AnimatePresence/],
        versions: [],
      },
      {
        name: "Embla Carousel",
        markers: [/EmblaCarousel|embla-carousel/],
        versions: [],
      },
      {
        name: "Vite/Rolldown runtime",
        markers: [/__vite__mapDeps|modulepreload|rolldown/i],
        versions: [],
      },
      {
        name: "Vue",
        markers: [/__VUE__|createElementVNode|resolveComponent|\bnextTick\b.*\bcreateApp\b/],
        versions: [/Vue[^0-9]{0,40}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Angular",
        markers: [/ng\.probe|ɵɵdefineComponent|@angular|NgModuleFactory/],
        versions: [/@angular[^0-9]{0,40}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Svelte",
        markers: [/\bsvelte\b|create_fragment|\$\$invalidate|SvelteComponent/],
        versions: [],
      },
      {
        name: "jQuery",
        markers: [/jQuery|\bjquery\b/],
        versions: [/jquery[\/ ]v?([0-9]+\.[0-9]+\.[0-9]+)/i, /jQuery\.fn\.jquery\s*=\s*["'`]([0-9.]+)["'`]/],
      },
      {
        name: "Lodash",
        markers: [/lodash|VERSION\s*=\s*["'`][0-9.]+["'`].*lodash|_\.runInContext/],
        versions: [/lodash[^0-9]{0,40}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Next.js",
        markers: [/__NEXT_DATA__|__next_f|next\/dist|_N_E\b/],
        versions: [/"buildId"|next\/dist[^0-9]{0,40}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Redux",
        markers: [/@@redux\/INIT|combineReducers|createStore.*dispatch/],
        versions: [],
      },
      {
        name: "D3",
        markers: [/d3-selection|d3-scale|\bd3\b.*version/],
        versions: [/d3[^0-9]{0,20}v?([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "GSAP",
        markers: [/gsap|TweenMax|TimelineLite|ScrollTrigger/],
        versions: [/gsap[^0-9]{0,20}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
      {
        name: "Axios",
        markers: [/axios|isAxiosError|createInstance.*defaults/],
        versions: [/axios[^0-9]{0,20}([0-9]+\.[0-9]+\.[0-9]+)/i],
      },
    ];
    const inventory = new Map();
    const items = state.items.filter(item =>
      item.bucket === "generated" &&
      item.resourceType === "script" &&
      typeof item.content === "string"
    );

    function record(detector, item, markerMatch, versionMatch) {
      const existing = inventory.get(detector.name) || {
        name: detector.name,
        version: null,
        confidence: "inferred",
        evidence: [],
      };
      let version = versionMatch && versionMatch.value ? versionMatch.value : null;
      if (version && detector.versionPrefix && !version.startsWith(detector.versionPrefix)) {
        version = detector.versionPrefix + version;
      }
      if (version && !existing.version) {
        existing.version = version;
        existing.confidence = "exact";
      } else if (!existing.version && detector.inferredVersion) {
        existing.version = detector.inferredVersion;
      }
      if (existing.evidence.length < 8) {
        existing.evidence.push({
          url: item.url,
          source: item.source,
          matched: versionMatch ? versionMatch.matched : markerMatch.matched,
          version: version || null,
        });
      }
      inventory.set(detector.name, existing);
    }

    for (const item of items) {
      for (const detector of detectors) {
        const markerMatch = captureMatch(detector.markers, item.content);
        if (!markerMatch) {
          continue;
        }
        const versionMatch = detector.versions.length
          ? captureMatch(detector.versions, item.content)
          : null;
        record(detector, item, markerMatch, versionMatch);
      }
    }

    // Strongest SBoM signal: package names recovered from source-map `sources`
    // paths (node_modules/<pkg> or webpack:///./node_modules/<pkg>). These name
    // dependencies precisely even when no runtime marker is present.
    const packagePattern = /node_modules\/((?:@[\w.-]+\/)?[\w.-]+)/g;
    const sourceMapItems = state.items.filter(item =>
      (item.bucket === "source-map" || item.resourceType === "source-map") &&
      typeof item.content === "string"
    );
    for (const item of sourceMapItems) {
      let sources = [];
      try {
        const parsed = JSON.parse(item.content);
        sources = Array.isArray(parsed.sources)
          ? parsed.sources
          : (Array.isArray(parsed.sections)
            ? parsed.sections.flatMap(s => (s && s.map && Array.isArray(s.map.sources)) ? s.map.sources : [])
            : []);
      } catch (_error) {
        continue;
      }
      for (const source of sources) {
        let pmatch;
        packagePattern.lastIndex = 0;
        while ((pmatch = packagePattern.exec(String(source || "")))) {
          const pkg = pmatch[1];
          if (!pkg || /^\.+$/.test(pkg)) {
            continue;
          }
          const name = `npm:${pkg}`;
          const existing = inventory.get(name) || {
            name,
            version: null,
            confidence: "exact",
            evidence: [],
          };
          if (existing.evidence.length < 8 && !existing.evidence.some(e => e.url === item.url)) {
            existing.evidence.push({
              url: item.url,
              source: "source-map-sources",
              matched: String(source).slice(0, 160),
              version: null,
            });
          }
          inventory.set(name, existing);
        }
      }
    }

    return Array.from(inventory.values()).sort((a, b) => a.name.localeCompare(b.name));
  }

  function capabilityForResource(record) {
    const url = String(record.url || record.finalURL || record.finalUrl || "");
    const source = String(record.source || "");
    const phase = String(record.phase || "");
    if (/auth-required/.test(phase) || /cloudflare access/i.test(record.bodyPrefix || "")) {
      return "auth-protected";
    }
    if (source === "service-worker-precache") {
      return "service-worker-precache";
    }
    if (/^vite-|vite-manifest|js-dynamic-import|js-static-import|js-side-effect-import|js-import-meta-url/.test(source)) {
      return "js-asset-graph";
    }
    if (/runtime-blob|runtime-worker|worker/i.test(source)) {
      return "worker";
    }
    if (record.resourceType === "wasm" || /runtime-wasm|\.wasm(?:[?#]|$)/i.test(source + " " + url)) {
      return "wasm";
    }
    if (/three-(?:draco|ktx2)-loader/.test(source) || /(?:draco_decoder|basis_transcoder)/i.test(url)) {
      return "three-loader";
    }
    if (/mapbox.*(?:draco|meshopt|worker|wasm)|(?:draco_decoder_gltf|meshopt_)/i.test(url)) {
      return "mapbox-runtime";
    }
    if (record.resourceType === "source-map" || source === "source-map" || /\.map(?:[?#]|$)/i.test(url)) {
      return "source-map";
    }
    return "";
  }

  function buildCapabilityDependencies() {
    const byKey = new Map();

    function add(record, status) {
      const capability = capabilityForResource(record);
      const url = record.url || record.finalURL || record.finalUrl || "";
      if (!capability || !url) {
        return;
      }
      const key = `${capability}\n${url}`;
      const existing = byKey.get(key) || {
        capability,
        url,
        status,
        resourceType: record.resourceType || null,
        source: record.source || null,
        discoveredFrom: record.discoveredFrom || null,
        generatedUrl: record.generatedUrl || record.generatedURL || null,
        credentialMode: record.credentialMode || null,
        requestHeaderNames: record.requestHeaderNames || [],
        errorPhase: null,
        errorMessage: null,
      };
      if (status === "error") {
        existing.status = "error";
        existing.errorPhase = record.phase || null;
        existing.errorMessage = record.message || null;
      }
      byKey.set(key, existing);
    }

    for (const item of state.items) {
      add(item, "captured");
    }
    for (const error of state.resourceErrors) {
      add(error, "error");
    }
    return Array.from(byKey.values()).sort((a, b) =>
      a.capability.localeCompare(b.capability) || a.url.localeCompare(b.url)
    );
  }

  async function installHook() {
    setBusy(true);
    try {
      const result = await devtoolsEvalFunction(installRuntimeHookInPage);
      if (result.alreadyInstalled) {
        log(`Runtime hook already installed; ${result.eventCount} events buffered.`);
      } else {
        log(`Runtime hook installed; ${result.eventCount} initial events buffered.`);
      }
    } catch (error) {
      log(`Runtime hook failed: ${error.message}`);
    } finally {
      setBusy(false);
    }
  }

  async function reloadAndCapture() {
    // Reload the inspected page with the runtime hook installed at
    // document-start, so Worker/WASM/Blob constructions and script/style loads
    // that happen *before* the panel was opened are instrumented from the very
    // first byte. Network capture (already listening) records the actual bodies.
    try {
      const injectedScript = `(${installRuntimeHookInPage.toString()})();`;
      // Reset capture buffer so the post-reload load is what we measure.
      state.network.byUrl.clear();
      state.network.totalBytes = 0;
      state.network.captured = 0;
      state.network.truncated = false;
      updateCapturedStat();
      browser.devtools.inspectedWindow.reload({
        injectedScript,
        ignoreCache: true,
      });
      log("Reloading page with the runtime hook injected at document-start; network capture is recording. Scan when the page settles.");
    } catch (error) {
      log(`Reload & Capture failed: ${error.message}`);
    }
  }

  async function scan() {
    setBusy(true);
    state.items = [];
    state.resourceErrors = [];
    state.serviceWorkerAssets = [];
    state.frameworkInventory = [];
    state.pageSnapshot = null;
    state.scanOptions = null;
    state.scanStartedAt = new Date().toISOString();
    state.scanFinishedAt = null;
    render();

    const scanOptions = options();
    state.scanOptions = scanOptions;
    try {
      if (scanOptions.preferNetworkCapture && state.network.captured) {
        log(`Network capture has ${state.network.captured} response(s); preferring captured bytes over re-fetch.`);
      } else if (scanOptions.preferNetworkCapture) {
        log("Network capture preferred but empty; reload the page (or use Reload & Capture) before scanning to record responses.");
      }

      if (scanOptions.preferNetworkCapture && state.network.captured) {
        log(`Network capture: ${state.network.captured} response(s) buffered; preferring them over re-fetch.`);
      } else if (scanOptions.preferNetworkCapture) {
        log("Network capture empty; reload the page (or use Reload & Capture) to record responses, else re-fetching.");
      }

      log("Collecting page runtime snapshot.");
      const snapshot = await devtoolsEvalFunction(collectPageSnapshotInPage);
      state.pageSnapshot = snapshot;
      els.targetLabel.textContent = snapshot.href || "Current target";
      if (Array.isArray(snapshot.frames) && snapshot.frames.length) {
        const accessible = snapshot.frames.filter(f => f.accessible).length;
        log(`Frames: ${snapshot.frames.length} (${accessible} same-origin walked, ${snapshot.frames.length - accessible} cross-origin captured by URL).`);
      }
      if (snapshot.truncation) {
        const flags = Object.keys(snapshot.truncation).filter(k => snapshot.truncation[k]);
        if (flags.length) {
          log(`Snapshot truncated (${flags.join(", ")}); full bytes recovered via fetch/network-capture.`);
        }
      }
      await collectInlineResources(snapshot);

      log("Collecting generated resource contents.");
      await collectGeneratedResourcesFromPageSnapshot(snapshot, scanOptions);

      if (scanOptions.prettyFallback) {
        addPrettyFallbacks();
      }

      state.frameworkInventory = detectFrameworkInventory();
      if (state.frameworkInventory.length) {
        log(`Detected framework/library signals: ${state.frameworkInventory.map(item => item.name).join(", ")}.`);
      }

      state.scanFinishedAt = new Date().toISOString();
      log(`Scan complete: ${state.items.length} dump entries; ${state.resourceErrors.length} fetch/source-map issue(s).`);
    } catch (error) {
      log(`Scan failed: ${error.message}`);
    } finally {
      render();
      setBusy(false);
    }
  }

  function sanitizeSegment(segment) {
    return String(segment || "unnamed")
      .replace(/%/g, "_")
      .replace(/[<>:"\\|?*\x00-\x1f]/g, "_")
      .replace(/^\.+$/, "_")
      .slice(0, 160);
  }

  function extensionForItem(item) {
    try {
      const parsed = new URL(item.url);
      const match = parsed.pathname.match(/(\.[a-z0-9]+)$/i);
      if (match) {
        return match[1];
      }
    } catch (_error) {
      // Fall back to type-based extensions below.
    }
    if (item.resourceType === "stylesheet") {
      return ".css";
    }
    if (item.resourceType === "source-map") {
      return ".map";
    }
    if (item.resourceType === "json") {
      return ".json";
    }
    if (item.resourceType === "wasm") {
      return ".wasm";
    }
    if (item.resourceType === "font" || item.resourceType === "image" || item.resourceType === "media") {
      return ".bin";
    }
    return ".js";
  }

  function pathFromURL(url, fallbackExt) {
    if (/^inline:\/\//.test(url)) {
      return url.replace(/^inline:\/\//, "inline/").replace(/\/+/g, "/");
    }
    if (/^webpack:\/\//.test(url)) {
      const rest = url.replace(/^webpack:\/+/, "webpack/");
      return rest.split("/").filter(Boolean).map(sanitizeSegment).join("/");
    }
    try {
      const parsed = new URL(url);
      const host = sanitizeSegment(parsed.host || parsed.protocol.replace(/:$/, ""));
      const rawPath = decodeURIComponent(parsed.pathname || "/index");
      const parts = rawPath.split("/").filter(Boolean).map(sanitizeSegment);
      if (!parts.length || rawPath.endsWith("/")) {
        parts.push("index" + fallbackExt);
      }
      let path = [host, ...parts].join("/");
      if (!/\.[a-z0-9]+$/i.test(path)) {
        path += fallbackExt;
      }
      return path;
    } catch (_error) {
      let path = sanitizeSegment(url).replace(/_+/g, "_");
      if (!/\.[a-z0-9]+$/i.test(path)) {
        path += fallbackExt;
      }
      return path;
    }
  }

  function uniquePath(path, used) {
    let candidate = path;
    let index = 2;
    while (used.has(candidate)) {
      const dot = path.lastIndexOf(".");
      if (dot > 0) {
        candidate = `${path.slice(0, dot)}_${index}${path.slice(dot)}`;
      } else {
        candidate = `${path}_${index}`;
      }
      index++;
    }
    used.add(candidate);
    return candidate;
  }

  function buildZipFiles() {
    const used = new Set();
    const files = [];
    const dumpOptions = state.scanOptions || options();
    const manifestItems = [];

    function addFile(path, content) {
      files.push({
        path: uniquePath(path, used),
        content,
      });
    }

    addFile("mindemon-manifest.json", JSON.stringify({
      tool: "mindemon-runtime-dumper",
      version: "0.1.0",
      target: state.pageSnapshot && state.pageSnapshot.href,
      scanStartedAt: state.scanStartedAt,
      scanFinishedAt: state.scanFinishedAt,
      options: redactedOptions(dumpOptions),
    }, null, 2));

    if (state.pageSnapshot) {
      addFile("meta/page-snapshot.json", JSON.stringify(state.pageSnapshot, null, 2));
      addFile(
        "meta/runtime-events.json",
        JSON.stringify(
          state.pageSnapshot.runtimeHook || { eventCount: 0, events: [] },
          null,
          2
        )
      );
    }
    addFile("meta/errors.json", JSON.stringify(state.resourceErrors, null, 2));
    addFile("meta/frameworks.json", JSON.stringify(state.frameworkInventory || [], null, 2));
    addFile("meta/libraries.json", JSON.stringify(state.frameworkInventory || [], null, 2));
    addFile("meta/service-worker-assets.json", JSON.stringify(state.serviceWorkerAssets || [], null, 2));
    addFile("meta/capability-dependencies.json", JSON.stringify(buildCapabilityDependencies(), null, 2));

    for (const item of state.items) {
      if (item.bucket === "generated" && !dumpOptions.includeGenerated) {
        continue;
      }
      const fallbackExt = extensionForItem(item);
      const base = pathFromURL(item.url, fallbackExt);
      const path = `${item.bucket}/${base}`;
      addFile(path, item.content);
      manifestItems.push({
        bucket: item.bucket,
        resourceType: item.resourceType,
        url: item.url,
        generatedUrl: item.generatedUrl || null,
        mapUrl: item.mapUrl || null,
        source: item.source,
        discoveredFrom: item.discoveredFrom || null,
        contentType: item.contentType || null,
        contentSource: item.contentSource || null,
        status: item.status == null ? null : item.status,
        statusText: item.statusText || null,
        finalUrl: item.finalUrl || null,
        credentialMode: item.credentialMode || null,
        requestHeaderNames: Array.isArray(item.requestHeaderNames)
          ? item.requestHeaderNames
          : [],
        manifestField: item.manifestField || null,
        manifestEntry: item.manifestEntry || null,
        viteDepIndex: item.viteDepIndex == null ? null : item.viteDepIndex,
        precacheRevision: item.precacheRevision || null,
        precacheIntegrity: item.precacheIntegrity || null,
        errors: item.errors || [],
        bytes: item.bytes,
        minified: item.minified,
      });
    }

    addFile("meta/resources.json", JSON.stringify(manifestItems, null, 2));
    return files;
  }

  async function downloadZip() {
    if (!state.items.length) {
      return;
    }
    const files = buildZipFiles();
    const blob = MindemonZip.createZip(files);
    const url = URL.createObjectURL(blob);
    const stamp = new Date().toISOString().replace(/[:.]/g, "-");
    const link = document.createElement("a");
    link.href = url;
    link.download = `mindemon-dump-${stamp}.zip`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30000);
    log(`Prepared ZIP with ${files.length} files.`);
  }

  // Map an internal resourceType to a representative content-type so the
  // backend's classifier (which prefers URL extension, then content-type) lands
  // on the right FileType even for extensionless URLs.
  function backendContentType(item) {
    if (item.contentType) {
      return item.contentType;
    }
    switch (item.resourceType) {
      case "stylesheet":
        return "text/css";
      case "json":
        return "application/json";
      case "source-map":
        return "application/json";
      case "html":
        return "text/html";
      case "script":
        return "application/javascript";
      default:
        return "application/octet-stream";
    }
  }

  // Build the /ingest/batch payload from the captured items. Text content is
  // sent as UTF-8; binary (Uint8Array) content is base64-encoded.
  function buildBackendPayload() {
    const files = [];
    // Index source maps by the generated file they belong to, so we can attach
    // them inline and let the backend reverse `sourcesContent` in one pass.
    const mapByGenerated = new Map();
    for (const item of state.items) {
      if (item.bucket === "source-map" && item.generatedUrl && typeof item.content === "string") {
        mapByGenerated.set(item.generatedUrl, item.content);
      }
    }
    for (const item of state.items) {
      // Originals are reconstructed by the backend from the source map; skip the
      // separate source-map bucket entries (attached inline below instead).
      if (item.bucket === "source-map") {
        continue;
      }
      const isText = typeof item.content === "string";
      const file = {
        url: item.url,
        content_type: backendContentType(item),
        content: isText ? item.content : bytesToBase64(bytesForContent(item.content)),
        encoding: isText ? "utf8" : "base64",
        source: "fs_ingest",
      };
      const mapContent = mapByGenerated.get(item.url);
      if (mapContent) {
        file.source_map_content = mapContent;
      }
      files.push(file);
    }
    return {
      target: state.pageSnapshot && state.pageSnapshot.href,
      files,
    };
  }

  async function sendToBackend() {
    if (!state.items.length) {
      return;
    }
    const base = (els.backendUrl.value || "http://127.0.0.1:3333").trim().replace(/\/+$/, "");
    setBusy(true);
    try {
      const payload = buildBackendPayload();
      log(`Sending ${payload.files.length} asset(s) to ${base}/ingest/batch ...`);
      const response = await fetch(`${base}/ingest/batch`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      if (!response.ok) {
        const text = await response.text();
        log(`Backend responded ${response.status}: ${text.slice(0, 300)}`);
        return;
      }
      const result = await response.json();
      log(`Backend ingested ${result.ingested} asset(s); ${result.totalFindings} total finding(s).`);
      const withFindings = (result.results || [])
        .filter(r => r.findings > 0)
        .slice(0, 12);
      for (const r of withFindings) {
        log(`  ${r.findings} finding(s) [${r.kind}] ${r.url}${r.reversedSources ? ` (+${r.reversedSources} reversed)` : ""}`);
      }
      const errors = (result.results || []).filter(r => r.error);
      if (errors.length) {
        log(`  ${errors.length} asset(s) errored; first: ${errors[0].error}`);
      }
    } catch (error) {
      log(`Send to backend failed: ${error.message}`);
      log("Start the daemon with: jsbeautify --serve   (default http://127.0.0.1:3333)");
    } finally {
      setBusy(false);
    }
  }

  async function dumpToLocalFolder() {
    if (!state.items.length) {
      return;
    }

    setBusy(true);
    let client = null;
    try {
      const files = buildZipFiles();
      client = connectNativeHost();
      const started = await nativeRequest(client, {
        action: "start",
        target: state.pageSnapshot && state.pageSnapshot.href,
      });
      log(`Native dump started: ${started.outputDir}`);

      for (const file of files) {
        await writeFileToNative(client, file);
      }

      const finished = await nativeRequest(client, { action: "finish" });
      log(`Dumped ${finished.fileCount} files to ${finished.outputDir}`);
    } catch (error) {
      log(`Local folder dump failed: ${error.message}`);
      log("Install the native host with: npm run install:native");
    } finally {
      if (client) {
        client.disconnect();
      }
      setBusy(false);
    }
  }

  function connectNativeHost() {
    const port = browser.runtime.connectNative("mindemon_dump_host");
    const pending = new Map();
    let nextId = 1;

    port.onMessage.addListener(message => {
      const request = pending.get(message.id);
      if (!request) {
        return;
      }
      pending.delete(message.id);
      if (message.ok === false) {
        request.reject(new Error(message.error || "Native host error"));
      } else {
        request.resolve(message);
      }
    });

    port.onDisconnect.addListener(() => {
      const error = new Error(
        browser.runtime.lastError
          ? browser.runtime.lastError.message
          : "Native host disconnected"
      );
      for (const request of pending.values()) {
        request.reject(error);
      }
      pending.clear();
    });

    return {
      post(message) {
        const id = nextId++;
        const request = { ...message, id };
        return new Promise((resolve, reject) => {
          pending.set(id, { resolve, reject });
          port.postMessage(request);
        });
      },
      disconnect() {
        port.disconnect();
      },
    };
  }

  function nativeRequest(client, message) {
    return client.post(message);
  }

  function bytesToBase64(bytes) {
    let binary = "";
    const step = 0x8000;
    for (let i = 0; i < bytes.length; i += step) {
      const chunk = bytes.subarray(i, i + step);
      binary += String.fromCharCode.apply(null, chunk);
    }
    return btoa(binary);
  }

  function base64ToBytes(base64) {
    const binary = atob(String(base64 || ""));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }

  async function writeFileToNative(client, file) {
    const bytes = bytesForContent(file.content);
    const chunkSize = 192 * 1024;
    await nativeRequest(client, {
      action: "startFile",
      path: file.path,
      size: bytes.length,
    });

    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      const chunk = bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length));
      await nativeRequest(client, {
        action: "writeChunk",
        data: bytesToBase64(chunk),
      });
    }

    await nativeRequest(client, { action: "finishFile" });
  }

  function render() {
    const generated = state.items.filter(item => item.bucket === "generated").length;
    const originals = state.items.filter(item => item.bucket === "original").length;
    const inline = state.items.filter(item => item.bucket === "inline").length;
    const events = state.pageSnapshot && state.pageSnapshot.runtimeHook
      ? state.pageSnapshot.runtimeHook.eventCount
      : 0;

    els.generatedCount.textContent = String(generated);
    els.originalCount.textContent = String(originals);
    els.inlineCount.textContent = String(inline);
    els.eventCount.textContent = String(events);
    if (els.capturedCount) {
      els.capturedCount.textContent = String(state.network.captured);
    }
    updateCapturedStat();
    els.dumpFolderButton.disabled = state.items.length === 0;
    els.downloadButton.disabled = state.items.length === 0;
    els.sendBackendButton.disabled = state.items.length === 0;

    if (!state.items.length) {
      els.resourceRows.innerHTML = '<tr><td colspan="5" class="empty">Run a scan to collect resources.</td></tr>';
      return;
    }

    els.resourceRows.textContent = "";
    for (const item of state.items) {
      const row = document.createElement("tr");
      row.innerHTML = `
        <td><span class="tag"></span></td>
        <td></td>
        <td></td>
        <td></td>
        <td></td>
      `;
      row.children[0].firstElementChild.textContent = item.bucket;
      row.children[1].textContent = item.resourceType;
      row.children[2].textContent = item.url;
      row.children[3].textContent = String(item.bytes);
      row.children[4].textContent = item.source;
      els.resourceRows.appendChild(row);
    }
  }

  function clear() {
    state.items = [];
    state.resourceErrors = [];
    state.serviceWorkerAssets = [];
    state.frameworkInventory = [];
    state.pageSnapshot = null;
    state.scanOptions = null;
    state.scanStartedAt = null;
    state.scanFinishedAt = null;
    els.targetLabel.textContent = "No scan yet";
    log("Cleared scan results.");
    render();
  }

  els.installHookButton.addEventListener("click", installHook);
  if (els.reloadButton) {
    els.reloadButton.addEventListener("click", reloadAndCapture);
  }
  els.scanButton.addEventListener("click", scan);
  els.dumpFolderButton.addEventListener("click", dumpToLocalFolder);
  els.downloadButton.addEventListener("click", downloadZip);
  els.sendBackendButton.addEventListener("click", sendToBackend);
  els.clearButton.addEventListener("click", clear);

  installNetworkCapture();
  render();
  log("Ready. Open this panel on a page, install the runtime hook if you want future load events, then scan.");
})();
