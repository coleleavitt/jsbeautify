(function (global) {
  "use strict";

  const SOURCE_MAPPING_RE =
    /(?:\/\/[#@]\s*sourceMappingURL=([^\s'"<>]+)|\/\*[#@]\s*sourceMappingURL=([^*]+?)\s*\*\/)/g;

  // Modern esbuild/Sentry builds frequently strip the `sourceMappingURL`
  // comment and instead embed a Debug ID (`//# debugId=<uuid>`), with the map
  // either uploaded out-of-band or available at `<generated>.map`. We detect the
  // id so we can (a) record it and (b) fall back to guessing the `.map` URL.
  const DEBUG_ID_RE =
    /(?:\/\/[#@]\s*debugId=([0-9a-fA-F-]{8,})|\/\*[#@]\s*debugId=([0-9a-fA-F-]{8,})\s*\*\/)/g;

  function extractSourceMappingURL(text) {
    let match;
    let last = null;
    while ((match = SOURCE_MAPPING_RE.exec(text))) {
      last = (match[1] || match[2] || "").trim();
    }
    return last;
  }

  function extractDebugId(text) {
    DEBUG_ID_RE.lastIndex = 0;
    let match;
    let last = null;
    while ((match = DEBUG_ID_RE.exec(text || ""))) {
      last = (match[1] || match[2] || "").trim();
    }
    return last;
  }

  // Guess the conventional sibling `.map` URL for a generated file, e.g.
  // https://x/app.123.js?v=2  ->  https://x/app.123.js.map
  function guessMapURL(generatedURL) {
    if (!generatedURL) {
      return null;
    }
    const noQuery = String(generatedURL).split(/[?#]/)[0];
    if (!noQuery) {
      return null;
    }
    return `${noQuery}.map`;
  }

  function isAbsoluteLike(url) {
    return /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url);
  }

  function originForURL(url) {
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
      return new URL(url).hostname.toLowerCase().replace(/\.$/, "");
    } catch (_error) {
      return "";
    }
  }

  // Curated subset of the Public Suffix List; kept in sync with panel.js.
  const MULTIPART_SUFFIXES = new Set([
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
    for (let start = 0; start < labels.length - 1; start++) {
      const suffix = labels.slice(start).join(".");
      if (MULTIPART_SUFFIXES.has(suffix) && start >= 1) {
        return labels.slice(start - 1).join(".");
      }
    }
    return labels.slice(-2).join(".");
  }

  function isFamilyOrigin(url, options) {
    if (!options || !options.allowFirstPartyFamily || !options.familyDomains) {
      return false;
    }
    let hostname;
    try {
      hostname = new URL(url).hostname.toLowerCase();
    } catch (_error) {
      return false;
    }
    const reg = registrableDomain(hostname);
    for (const domain of options.familyDomains) {
      if (reg === domain || hostname === domain || hostname.endsWith("." + domain)) {
        return true;
      }
    }
    return false;
  }

  function isOriginAllowed(url, options) {
    if (!options || options.allowExternalOrigins) {
      return true;
    }
    const origin = originForURL(url);
    if (!origin) {
      return true;
    }
    if (options.pageOrigin && origin === options.pageOrigin) {
      return true;
    }
    if (options.allowedOrigins && options.allowedOrigins.has(origin)) {
      return true;
    }
    return isFamilyOrigin(url, options) || isExplicitCfAccessTarget(url, options);
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

  function isExplicitCfAccessTarget(url, options) {
    const cf = options && options.cloudflareAccess;
    return Boolean(
      cf &&
        cf.clientId &&
        cf.clientSecret &&
        Array.isArray(cf.domains) &&
        cf.domains.length &&
        hostMatchesDomainList(hostnameForURL(url), cf.domains)
    );
  }

  function credentialsFor(url, options) {
    const origin = originForURL(url);
    if (!origin || !options) {
      return "omit";
    }
    if (
      options.useSessionCredentials &&
      (origin === options.pageOrigin ||
        (options.allowedOrigins && options.allowedOrigins.has(origin)) ||
        isFamilyOrigin(url, options) ||
        isExplicitCfAccessTarget(url, options))
    ) {
      return "include";
    }
    return "omit";
  }

  function shouldAttachCfAccessHeaders(url, options) {
    const cf = options && options.cloudflareAccess;
    if (!cf || !cf.clientId || !cf.clientSecret) {
      return false;
    }
    const host = hostnameForURL(url);
    if (!host) {
      return false;
    }
    if (Array.isArray(cf.domains) && cf.domains.length) {
      return hostMatchesDomainList(host, cf.domains);
    }
    const origin = originForURL(url);
    return Boolean(
      origin &&
        options &&
        (origin === options.pageOrigin ||
          (options.allowedOrigins && options.allowedOrigins.has(origin)) ||
          isFamilyOrigin(url, options))
    );
  }

  function requestHeadersFor(url, options) {
    if (!shouldAttachCfAccessHeaders(url, options)) {
      return {};
    }
    return {
      "CF-Access-Client-Id": options.cloudflareAccess.clientId,
      "CF-Access-Client-Secret": options.cloudflareAccess.clientSecret,
    };
  }

  function requestHeaderNames(headers) {
    return Object.keys(headers || {}).sort();
  }

  function resolveURL(ref, baseURL) {
    if (!ref) {
      return null;
    }
    if (isAbsoluteLike(ref)) {
      return ref;
    }
    try {
      return new URL(ref, baseURL).href;
    } catch (_error) {
      return ref;
    }
  }

  function decodeDataURL(url) {
    const match = /^data:([^,]*),(.*)$/i.exec(url);
    if (!match) {
      throw new Error("Invalid data URL");
    }
    const meta = match[1] || "";
    const payload = match[2] || "";
    if (/;base64/i.test(meta)) {
      const binary = atob(payload);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
      }
      return new TextDecoder().decode(bytes);
    }
    return decodeURIComponent(payload);
  }

  function contentTypeForDataURL(url) {
    const match = /^data:([^,]*),(.*)$/i.exec(url);
    if (!match) {
      return "";
    }
    return (match[1] || "").split(";")[0] || "text/plain";
  }

  function bodyPrefix(text) {
    return String(text || "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 500);
  }

  function isHTMLPayload(contentType, text) {
    if (/^\s*text\/html\b/i.test(contentType || "")) {
      return true;
    }
    const head = String(text || "").slice(0, 512).toLowerCase();
    return /^\s*<!doctype html/.test(head) || /^\s*<html[\s>]/.test(head);
  }

  function isAuthHTMLPayload(text, finalURL) {
    const haystack = `${finalURL || ""}\n${String(text || "").slice(0, 4000)}`.toLowerCase();
    return /cloudflare access|cf-access-domain|\/cdn-cgi\/access\/login|sign in .*cloudflare access/.test(haystack);
  }

  function classifyUnexpectedHTML(details, expected) {
    if (!details || !isHTMLPayload(details.contentType, details.text)) {
      return null;
    }
    if (isAuthHTMLPayload(details.text, details.finalURL)) {
      return {
        phase: expected === "source-map" ? "load-map-auth-required" : "fetch-original-auth-required",
        message: `Received an authentication HTML page instead of ${expected}`,
      };
    }
    return {
      phase: expected === "source-map" ? "load-map-html-response" : "fetch-original-html-response",
      message: `Received HTML instead of ${expected}`,
    };
  }

  function detailsForError(error, fallbackURL, fallbackDetails) {
    const details = (error && error.details) || fallbackDetails || {};
    return {
      status: details.status == null ? null : details.status,
      statusText: details.statusText || "",
      contentType: details.contentType || "",
      finalURL: details.finalURL || fallbackURL || "",
      bodyPrefix: details.bodyPrefix || "",
      credentialMode: details.credentialMode || "",
      requestHeaderNames: Array.isArray(details.requestHeaderNames)
        ? details.requestHeaderNames.slice()
        : [],
    };
  }

  function pushError(errors, phase, url, message, details) {
    errors.push({
      phase,
      url,
      message,
      ...detailsForError(null, url, details),
    });
  }

  async function fetchTextDetails(url, credentials, headers) {
    if (/^data:/i.test(url)) {
      const text = decodeDataURL(url);
      return {
        text,
        status: 0,
        statusText: "data-url",
        contentType: contentTypeForDataURL(url),
        finalURL: url,
        bodyPrefix: bodyPrefix(text),
        credentialMode: "omit",
        requestHeaderNames: [],
      };
    }
    if (!/^https?:|^file:|^moz-extension:/i.test(url)) {
      throw new Error("Unsupported URL scheme for fetch: " + url);
    }
    const credentialMode = credentials || "include";
    const response = await fetch(url, {
      credentials: credentialMode,
      cache: "no-store",
      headers: headers || {},
    });
    const text = await response.text();
    const details = {
      text,
      status: response.status,
      statusText: response.statusText || "",
      contentType: response.headers.get("content-type") || "",
      finalURL: response.url || url,
      bodyPrefix: bodyPrefix(text),
      credentialMode,
      requestHeaderNames: requestHeaderNames(headers),
    };
    if (!response.ok) {
      const error = new Error("HTTP " + response.status + " for " + url);
      error.details = details;
      throw error;
    }
    return details;
  }

  async function fetchText(url, credentials) {
    const details = await fetchTextDetails(url, credentials);
    return details.text;
  }

  function resolveOriginalURL(source, sourceRoot, mapURL) {
    if (!source) {
      return mapURL;
    }
    if (isAbsoluteLike(source)) {
      return source;
    }
    let base = mapURL;
    if (sourceRoot) {
      try {
        base = new URL(sourceRoot, mapURL).href;
      } catch (_error) {
        base = sourceRoot;
      }
    }
    return resolveURL(source, base) || source;
  }

  async function collectOriginals(map, mapURL, generatedURL, options, errors, sectionIndex) {
    if (!map || typeof map !== "object") {
      return [];
    }

    if (Array.isArray(map.sections)) {
      const nested = [];
      for (let i = 0; i < map.sections.length; i++) {
        const section = map.sections[i];
        let childMap = section && section.map;
        let childMapURL = mapURL;
        // Index maps may reference an external section map via `url` instead of
        // inlining it under `map`.
        if (!childMap && section && typeof section.url === "string") {
          const sectionURL = resolveURL(section.url, mapURL);
          if (!/^data:/i.test(sectionURL) && !isOriginAllowed(sectionURL, options)) {
            errors.push({
              phase: "skip-section-map-out-of-scope",
              url: sectionURL,
              message: "External origin; enable deep crawl to fetch this section source map",
            });
            continue;
          }
          try {
            const sectionDetails = await fetchTextDetails(
              sectionURL,
              credentialsFor(sectionURL, options),
              requestHeadersFor(sectionURL, options)
            );
            const htmlFailure = classifyUnexpectedHTML(sectionDetails, "source-map");
            if (htmlFailure) {
              pushError(errors, htmlFailure.phase, sectionURL, htmlFailure.message, sectionDetails);
              continue;
            }
            childMap = JSON.parse(sectionDetails.text);
            childMapURL = sectionURL;
          } catch (error) {
            errors.push({
              phase: "load-section-map",
              url: sectionURL,
              message: String(error && error.message ? error.message : error),
              ...detailsForError(error, sectionURL),
            });
            continue;
          }
        }
        nested.push(
          ...(await collectOriginals(
            childMap,
            childMapURL,
            generatedURL,
            options,
            errors,
            i
          ))
        );
      }
      return nested;
    }

    const sources = Array.isArray(map.sources) ? map.sources : [];
    const sourcesContent = Array.isArray(map.sourcesContent) ? map.sourcesContent : [];
    const originals = [];

    for (let i = 0; i < sources.length; i++) {
      const source = sources[i];
      const originalURL = resolveOriginalURL(source, map.sourceRoot, mapURL);
      let content = sourcesContent[i];
      let contentSource = content == null ? "missing" : "sourcesContent";

      if (content == null && options.fetchMissingSources) {
        if (!isOriginAllowed(originalURL, options)) {
          errors.push({
            phase: "skip-original-out-of-scope",
            url: originalURL,
            message: "External origin; enable deep crawl to fetch this original",
          });
          contentSource = "skipped-out-of-scope";
        } else {
          try {
            const fetched = await fetchTextDetails(
              originalURL,
              credentialsFor(originalURL, options),
              requestHeadersFor(originalURL, options)
            );
            const htmlFailure = classifyUnexpectedHTML(fetched, "original source");
            if (htmlFailure) {
              pushError(errors, htmlFailure.phase, originalURL, htmlFailure.message, fetched);
              contentSource = htmlFailure.phase.endsWith("auth-required")
                ? "auth-required"
                : "html-response";
            } else {
              content = fetched.text;
              contentSource = "fetch";
            }
          } catch (error) {
            errors.push({
              phase: "fetch-original",
              url: originalURL,
              message: String(error && error.message ? error.message : error),
              ...detailsForError(error, originalURL),
            });
          }
        }
      }

      originals.push({
        source,
        url: originalURL,
        generatedURL,
        mapURL,
        sectionIndex: sectionIndex == null ? null : sectionIndex,
        content: content == null ? "" : String(content),
        contentSource,
      });
    }

    return originals;
  }

  async function resolveForText(resource, options) {
    options = options || {};
    options.pageOrigin = options.pageOrigin || originForURL(options.pageURL || "");
    const errors = [];
    // The `SourceMap:` (or legacy `X-SourceMap:`) HTTP response header is an
    // alternative to the `//# sourceMappingURL=` comment and takes precedence
    // when present, since the comment is frequently stripped from production
    // builds while the header is retained.
    const headerMappingURL = typeof resource.sourceMapHeader === "string"
      ? resource.sourceMapHeader.trim()
      : "";
    const explicitMappingURL = headerMappingURL || extractSourceMappingURL(resource.content || "");
    const debugId = extractDebugId(resource.content || "");
    // If there's no explicit sourceMappingURL (comment stripped) but the file
    // carries a debugId, fall back to the conventional `<generated>.map` sibling.
    const guessedMappingURL = !explicitMappingURL && debugId
      ? guessMapURL(resource.finalUrl || resource.finalURL || resource.url)
      : null;
    const sourceMappingURL = explicitMappingURL || guessedMappingURL;
    const mapSource = explicitMappingURL ? "sourceMappingURL" : (guessedMappingURL ? "debugId-guess" : null);
    if (!sourceMappingURL) {
      return {
        sourceMappingURL: null,
        mapURL: null,
        mapText: null,
        originals: [],
        debugId: debugId || null,
        mapSource: null,
        errors,
      };
    }

    const generatedURL = resource.url;
    const sourceMapBaseURL = resource.finalUrl || resource.finalURL || resource.url;
    const mapURL = resolveURL(sourceMappingURL, sourceMapBaseURL);
    let mapText;
    let map;
    let mapDetails = null;

    if (!/^data:/i.test(mapURL) && !isOriginAllowed(mapURL, options)) {
      errors.push({
        phase: "skip-map-out-of-scope",
        url: mapURL,
        message: "External origin; enable deep crawl to fetch this source map",
      });
      return {
        sourceMappingURL,
        mapURL,
        mapText: null,
        originals: [],
        debugId: debugId || null,
        mapSource,
        errors,
      };
    }

    try {
      mapDetails = await fetchTextDetails(
        mapURL,
        credentialsFor(mapURL, options),
        requestHeadersFor(mapURL, options)
      );
      const htmlFailure = classifyUnexpectedHTML(mapDetails, "source-map");
      if (htmlFailure) {
        // A guessed `.map` that doesn't exist is expected, not a real failure;
        // record it at a softer phase so it doesn't read as an error.
        const phase = mapSource === "debugId-guess" ? "debugid-guess-miss" : htmlFailure.phase;
        pushError(errors, phase, mapURL, htmlFailure.message, mapDetails);
        return {
          sourceMappingURL,
          mapURL,
          mapText: null,
          originals: [],
          debugId: debugId || null,
          mapSource,
          errors,
        };
      }
      mapText = mapDetails.text;
      map = JSON.parse(mapText);
    } catch (error) {
      const phase = mapSource === "debugId-guess" ? "debugid-guess-miss" : "load-map";
      errors.push({
        phase,
        url: mapURL,
        message: String(error && error.message ? error.message : error),
        ...detailsForError(error, mapURL, mapDetails),
      });
      return {
        sourceMappingURL,
        mapURL,
        mapText: null,
        originals: [],
        debugId: debugId || null,
        mapSource,
        errors,
      };
    }

    const originals = await collectOriginals(
      map,
      mapURL,
      generatedURL,
      options,
      errors,
      null
    );

    return {
      sourceMappingURL,
      mapURL,
      mapText,
      originals,
      debugId: debugId || null,
      mapSource,
      errors,
    };
  }

  global.MindemonSourceMaps = {
    extractSourceMappingURL,
    extractDebugId,
    guessMapURL,
    fetchTextDetails,
    resolveForText,
  };
})(globalThis);
