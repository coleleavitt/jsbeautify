//! Recursive recon engine — the "recon brain".
//!
//! Chains the building blocks into a closed discovery loop:
//!
//! ```text
//!   seed domain
//!      │  crt.sh CT-log enumeration  (domain_recon)
//!      ▼
//!   subdomains ──probe──► live hosts
//!      │  fetch "/" + referenced JS/HTML/source-maps
//!      ▼
//!   assets ──beautify+deobfuscate on ingest──► clean source   (Pipeline)
//!      │  run analyzers
//!      ▼
//!   findings (hostnames, urls, paths, secrets)
//!      │  extract NEW in-scope hosts/endpoints
//!      ▼
//!   feed back into the frontier ──────────────────────────────┐
//!      └───────────────────────── loop until frontier empty ──┘
//! ```
//!
//! Everything is scope-bounded (registrable-domain match) so the frontier can't
//! explode across unrelated origins, and deduplicated by URL + content hash so
//! the same bundle served from many paths isn't re-processed.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::time::Duration;

use serde::Serialize;

use crate::analyzers::match_kind::FileType;
use crate::analyzers::{AnalyzerConfig, run_all};
use crate::domain_recon::{self, ReconConfig};
use crate::scope::ScopeMatcher;

/// Configuration for a recursive recon run.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Registrable seed domain, e.g. `blackrock.com`.
    pub domain: String,
    /// Max recursion rounds (frontier-expansion generations).
    pub max_rounds: usize,
    /// Max hosts to fetch per round (politeness / blast-radius cap).
    pub max_hosts_per_round: usize,
    /// Max total assets to fetch across the whole run.
    pub max_assets: usize,
    /// Per-request HTTP timeout, seconds.
    pub timeout_secs: u64,
    /// Concurrent fetches.
    pub concurrency: usize,
    /// Run the deobfuscator on fetched JS before analysis.
    pub deobfuscate: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            domain: String::new(),
            max_rounds: 2,
            max_hosts_per_round: 50,
            max_assets: 500,
            timeout_secs: 15,
            concurrency: 16,
            deobfuscate: true,
        }
    }
}

/// A single fetched + analyzed asset.
#[derive(Debug, Clone, Serialize)]
pub struct AssetReport {
    pub url: String,
    pub status: u16,
    pub bytes: usize,
    pub findings: usize,
    /// Whether a `//# sourceMappingURL` was found (and queued).
    pub has_source_map: bool,
}

/// Final result of a recursive recon run.
#[derive(Debug, Clone, Serialize, Default)]
pub struct EngineReport {
    pub domain: String,
    pub rounds_run: usize,
    /// Every in-scope host discovered (CT logs + extracted from findings).
    pub hosts: Vec<String>,
    /// Live hosts that answered an HTTP request.
    pub live_hosts: Vec<String>,
    /// Assets fetched + analyzed.
    pub assets: Vec<AssetReport>,
    /// New endpoints/paths discovered inside JS (deduplicated).
    pub endpoints: Vec<String>,
    /// Secret-ish findings (kind contains `secret`/`api_key`/`token`).
    pub secrets: Vec<String>,
    pub total_findings: usize,
}

/// URL of a `.js`/`.mjs` asset referenced from HTML `src="…"`.
fn extract_script_srcs(html: &str, base_url: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Cheap, dependency-free scan for src="..."/src='...' on <script>.
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut idx = 0;
    while let Some(rel) = lower[idx..].find("<script") {
        let start = idx + rel;
        let end = lower[start..].find('>').map_or(html.len(), |e| start + e);
        let tag = &html[start..end.min(bytes.len())];
        if let Some(abs) = attr_value(tag, "src").and_then(|src| resolve_url(&src, base_url)) {
            out.push(abs);
        }
        idx = end;
    }
    out
}

/// Pull `name="value"` (single or double quoted) out of a tag fragment.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let at = lower.find(&key)? + key.len();
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let endq = rest[1..].find(quote)? + 1;
        Some(rest[1..endq].to_string())
    } else {
        // Unquoted: read until whitespace or end.
        let endw = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        Some(rest[..endw].to_string())
    }
}

/// Resolve a possibly-relative URL against a base.
fn resolve_url(href: &str, base_url: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with("data:") || href.starts_with("javascript:") {
        return None;
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base = url::Url::parse(base_url).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

/// Find a `//# sourceMappingURL=` reference in JS and resolve it.
fn extract_source_map_url(js: &str, base_url: &str) -> Option<String> {
    let marker = "sourceMappingURL=";
    let at = js.rfind(marker)? + marker.len();
    let rest = &js[at..];
    let end = rest.find(['\n', '\r', ' ', '*']).unwrap_or(rest.len());
    let raw = rest[..end].trim();
    if raw.is_empty() || raw.starts_with("data:") {
        return None;
    }
    resolve_url(raw, base_url)
}

/// Classify a finding kind as a "secret-ish" signal worth surfacing.
fn is_secret_kind(kind: &str) -> bool {
    kind.contains("secret")
        || kind.contains("api_key")
        || kind.contains("apikey")
        || kind.contains("token")
        || kind.contains("password")
        || kind.contains("private_key")
        || kind == "google_api_key"
        || kind == "aws_access_key"
        || kind == "basic_auth_url"
}

/// Mutable state threaded through a single recon run.
#[derive(Default)]
struct RunState {
    known_hosts: BTreeSet<String>,
    host_frontier: VecDeque<String>,
    fetched_urls: HashSet<String>,
    endpoints: BTreeSet<String>,
    secrets: BTreeSet<String>,
    live_hosts: BTreeSet<String>,
}

/// The recon engine.
pub struct ReconEngine {
    cfg: EngineConfig,
    scope: ScopeMatcher,
    client: reqwest::Client,
}

impl ReconEngine {
    /// Build an engine for `cfg`. Scope is the seed registrable domain (so only
    /// `*.domain` hosts are ever fetched / fed back).
    ///
    /// # Errors
    /// Fails if the HTTP client cannot be constructed.
    pub fn new(cfg: EngineConfig) -> Result<Self, String> {
        let in_scope = format!("*.{}", cfg.domain);
        let scope = ScopeMatcher::new(&[in_scope.as_str(), cfg.domain.as_str()], &[]);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("js-beautify-rs/recon-engine")
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        Ok(Self { cfg, scope, client })
    }

    /// Run the full recursive loop and return a report.
    ///
    /// # Errors
    /// Propagates the initial crt.sh enumeration error; per-asset fetch/analyze
    /// failures are recorded, not fatal.
    pub async fn run(&self) -> Result<EngineReport, String> {
        let mut report = EngineReport {
            domain: self.cfg.domain.clone(),
            ..EngineReport::default()
        };

        // Round 0: CT-log subdomain enumeration seeds the host frontier.
        let recon_cfg = ReconConfig {
            domain: self.cfg.domain.clone(),
            probe: false,
            ..ReconConfig::default()
        };
        // Fuse all passive sources (crt.sh + Wayback) so the frontier is seeded
        // from more than one catalog.
        let seed_hosts = domain_recon::fetch_all_sources(&self.client, &recon_cfg).await?;

        let mut st = RunState::default();
        st.known_hosts = seed_hosts.into_iter().collect();
        st.host_frontier = st.known_hosts.iter().cloned().collect();

        for round in 0..self.cfg.max_rounds {
            if st.host_frontier.is_empty() || report.assets.len() >= self.cfg.max_assets {
                break;
            }
            report.rounds_run = round + 1;

            // Take a politeness-bounded batch of hosts this round.
            let batch: Vec<String> = (0..self.cfg.max_hosts_per_round)
                .filter_map(|_| st.host_frontier.pop_front())
                .collect();
            if batch.is_empty() {
                break;
            }
            for host in batch {
                if report.assets.len() >= self.cfg.max_assets {
                    break;
                }
                self.process_host(&host, &mut st, &mut report).await;
            }
        }

        report.hosts = st.known_hosts.into_iter().collect();
        report.live_hosts = st.live_hosts.into_iter().collect();
        report.endpoints = st.endpoints.into_iter().collect();
        report.secrets = st.secrets.into_iter().collect();
        Ok(report)
    }

    /// Fetch a host's root page, analyze it, and fetch+analyze its referenced
    /// scripts (and their source maps).
    async fn process_host(&self, host: &str, st: &mut RunState, report: &mut EngineReport) {
        let root = format!("https://{host}/");
        let Some((status, body, ctype)) = self.fetch(&root).await else {
            return;
        };
        st.live_hosts.insert(host.to_string());

        // Collect in-scope <script src> URLs to fetch after the root.
        let mut script_urls: Vec<String> = Vec::new();
        if ctype.contains("html") || body.contains("<script") {
            for src in extract_script_srcs(&body, &root) {
                if self.in_scope_url(&src) {
                    script_urls.push(src);
                }
            }
        }

        // Analyze the root response itself.
        self.process_asset(&root, status, &body, st, report);

        for url in script_urls {
            if report.assets.len() >= self.cfg.max_assets {
                break;
            }
            if !st.fetched_urls.insert(url.clone()) {
                continue;
            }
            let Some((s, b, _ct)) = self.fetch(&url).await else {
                continue;
            };
            // Source-map discovery: fetch + analyze the .map too (recovers originals).
            self.try_fetch_source_map(&b, &url, st, report).await;
            self.process_asset(&url, s, &b, st, report);
        }
    }

    /// If `js` references an in-scope, not-yet-seen source map, fetch + analyze
    /// it (recovers original source). No-op otherwise.
    async fn try_fetch_source_map(&self, js: &str, base_url: &str, st: &mut RunState, report: &mut EngineReport) {
        let Some(map_url) =
            extract_source_map_url(js, base_url).filter(|m| self.in_scope_url(m) && st.fetched_urls.insert(m.clone()))
        else {
            return;
        };
        if let Some((ms, mb, _)) = self.fetch(&map_url).await {
            self.process_asset(&map_url, ms, &mb, st, report);
        }
    }

    fn in_scope_url(&self, url: &str) -> bool {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| self.scope.is_in_scope(h)))
            .unwrap_or(false)
    }

    /// Fetch a URL, returning `(status, body, content_type)` on success.
    async fn fetch(&self, url: &str) -> Option<(u16, String, String)> {
        let resp = self.client.get(url).send().await.ok()?;
        let status = resp.status().as_u16();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let body = resp.text().await.ok()?;
        Some((status, body, ctype))
    }

    /// Deobfuscate (optionally), analyze, and harvest new hosts/endpoints from
    /// one asset body. Mutates the run state + report in place.
    fn process_asset(&self, url: &str, status: u16, body: &str, st: &mut RunState, report: &mut EngineReport) {
        let file_type = FileType::from_path(url);

        // Deobfuscate JS before analysis when enabled (best-effort).
        let analyzed: String = if self.cfg.deobfuscate && matches!(file_type, FileType::Js) {
            let mut deob = crate::ast_deobfuscate::AstDeobfuscator::new();
            deob.deobfuscate(body).unwrap_or_else(|_| body.to_string())
        } else {
            body.to_string()
        };

        let matches = run_all(&analyzed, Some(url), file_type, &AnalyzerConfig::default());
        let finding_count = matches.len();
        report.total_findings += finding_count;

        for m in &matches {
            let kind = m.match_kind.as_str();
            if matches!(kind, "hostname" | "comment_hostname") {
                // New in-scope hostnames feed the frontier (scope-gated + dedup).
                let host = m.match_value.trim().to_ascii_lowercase();
                if self.scope.is_in_scope(&host) && st.known_hosts.insert(host.clone()) {
                    st.host_frontier.push_back(host);
                }
            } else if matches!(kind, "url" | "path" | "comment_path" | "api_call") {
                st.endpoints.insert(m.match_value.clone());
            } else if is_secret_kind(kind) {
                st.secrets.insert(format!("[{kind}] {}", m.match_value));
            }
            // Other kinds carry no frontier/endpoint signal; intentionally skipped.
        }

        report.assets.push(AssetReport {
            url: url.to_string(),
            status,
            bytes: body.len(),
            findings: finding_count,
            has_source_map: extract_source_map_url(body, url).is_some(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_script_srcs_absolute_and_relative() {
        let html = r#"<html><head>
            <script src="/assets/app.js"></script>
            <script src="https://cdn.example.com/lib.js"></script>
            <script>inline()</script>
        </head></html>"#;
        let srcs = extract_script_srcs(html, "https://target.com/page");
        assert!(srcs.iter().any(|s| s == "https://target.com/assets/app.js"));
        assert!(srcs.iter().any(|s| s == "https://cdn.example.com/lib.js"));
        // Inline script (no src) is not captured.
        assert_eq!(srcs.len(), 2);
    }

    #[test]
    fn resolves_source_map_url() {
        let js = "console.log(1)\n//# sourceMappingURL=app.js.map";
        let map = extract_source_map_url(js, "https://t.com/static/app.js");
        assert_eq!(map.as_deref(), Some("https://t.com/static/app.js.map"));
    }

    #[test]
    fn ignores_data_uri_source_map() {
        let js = "x\n//# sourceMappingURL=data:application/json;base64,abc";
        assert!(extract_source_map_url(js, "https://t.com/a.js").is_none());
    }

    #[test]
    fn secret_kind_classification() {
        assert!(is_secret_kind("google_api_key"));
        assert!(is_secret_kind("sensitive_property_name:secret"));
        assert!(is_secret_kind("aws_token"));
        assert!(!is_secret_kind("hostname"));
        assert!(!is_secret_kind("path"));
    }

    #[test]
    fn scope_gates_out_of_family_hosts() {
        let engine = ReconEngine::new(EngineConfig {
            domain: "blackrock.com".to_string(),
            ..EngineConfig::default()
        })
        .unwrap();
        assert!(engine.in_scope_url("https://login.blackrock.com/x.js"));
        assert!(!engine.in_scope_url("https://evil.example.com/x.js"));
    }
}
