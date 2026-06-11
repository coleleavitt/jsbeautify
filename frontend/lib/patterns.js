// Canonical pattern/host helpers shared by panel.js and lib/source-map.js.
//
// registrableDomain + the public-suffix subset were previously duplicated in
// both files (with a "kept in sync with panel.js" comment that invited drift).
// This module is the single source of truth: it attaches to
// globalThis.MindemonPatterns for the extension and exports via module.exports
// for `node --test`. Consumers prefer it and keep a local fallback.
(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.MindemonPatterns = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  // Curated subset of the Public Suffix List. Multi-part suffixes where the
  // registrable domain is suffix + one more label (e.g. example.co.uk).
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

  function hostnameForURL(url) {
    try {
      return new URL(url).hostname.toLowerCase().replace(/\.$/, "");
    } catch (_error) {
      return "";
    }
  }

  // Compute the registrable ("eTLD+1") domain for a hostname using the curated
  // suffix list. IPs and `localhost` are returned as-is.
  function registrableDomain(hostname) {
    const host = String(hostname || "").toLowerCase().replace(/\.$/, "");
    if (!host || /^[0-9.]+$/.test(host) || host === "localhost") {
      return host;
    }
    const labels = host.split(".");
    if (labels.length <= 2) {
      return host;
    }
    // Longest-suffix match: the longest known multi-part suffix the host ends
    // with, then one more label is the registrable domain.
    for (let start = 0; start < labels.length - 1; start++) {
      const suffix = labels.slice(start).join(".");
      if (MULTIPART_SUFFIXES.has(suffix) && start >= 1) {
        return labels.slice(start - 1).join(".");
      }
    }
    return labels.slice(-2).join(".");
  }

  return { MULTIPART_SUFFIXES, hostnameForURL, registrableDomain };
});
