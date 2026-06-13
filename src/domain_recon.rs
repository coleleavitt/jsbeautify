//! Whole-domain reconnaissance via Certificate Transparency (crt.sh).
//!
//! The browser-extension capture path only ever sees assets the *inspected
//! page* references — so a capture of `host/authplatform/` mines that one
//! origin and nothing else. This module widens the net: given a registrable
//! domain (e.g. `blackrock.com`) it asks crt.sh for every hostname that has
//! ever appeared in a logged TLS certificate, deduplicates them, and
//! (optionally) probes each over HTTPS to see which are live and what they
//! serve. The result is a worklist of origins you can then feed back into the
//! beautify/analyze pipeline — covering the whole domain, not just one path.
//!
//! crt.sh is a free, public CT-log search front-end (no API key). We hit its
//! JSON endpoint: `https://crt.sh/?q=%25.<domain>&output=json`.

use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One certificate-transparency record as returned by crt.sh's JSON output.
/// Only the fields we use are modelled; crt.sh returns several more.
#[derive(Debug, Clone, Deserialize)]
struct CrtShEntry {
    /// Newline-or-comma separated list of names on the cert (CN + SANs).
    name_value: String,
    #[serde(default)]
    common_name: String,
}

/// A discovered subdomain plus optional live-probe result.
#[derive(Debug, Clone, Serialize)]
pub struct Subdomain {
    pub host: String,
    /// HTTPS status code if probed and reachable.
    pub status: Option<u16>,
    /// `Server` response header if probed.
    pub server: Option<String>,
    /// Final URL after redirects (reveals canonical hosts / SSO redirects).
    pub final_url: Option<String>,
    /// `content-type` of the response, when probed.
    pub content_type: Option<String>,
}

/// Configuration for a domain-recon run.
#[derive(Debug, Clone)]
pub struct ReconConfig {
    /// Registrable domain to enumerate (e.g. `blackrock.com`).
    pub domain: String,
    /// If true, send an HTTPS HEAD/GET to each host to see which are live.
    pub probe: bool,
    /// Per-request timeout, seconds.
    pub timeout_secs: u64,
    /// Max concurrent probes.
    pub concurrency: usize,
    /// Drop wildcard (`*.`) entries from the CT results.
    pub skip_wildcards: bool,
    /// Also enumerate via the Wayback Machine CDX index (fuses with crt.sh).
    pub use_wayback: bool,
}

impl Default for ReconConfig {
    fn default() -> Self {
        Self {
            domain: String::new(),
            // crt.sh responses for busy domains are multi-MB and slow; give the
            // CT fetch plenty of headroom. Probes use their own shorter budget.
            probe: false,
            timeout_secs: 60,
            concurrency: 20,
            skip_wildcards: true,
            use_wayback: false,
        }
    }
}

/// Outcome of a recon run.
#[derive(Debug, Clone, Serialize)]
pub struct ReconResult {
    pub domain: String,
    /// Every unique hostname discovered in CT logs (sorted).
    pub hosts: Vec<String>,
    /// Probe results (only populated when `probe` is set), sorted by host.
    pub live: Vec<Subdomain>,
}

/// Normalize a raw CT name into a clean lowercase hostname, or `None` to drop.
fn clean_host(raw: &str, skip_wildcards: bool) -> Option<String> {
    let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() {
        return None;
    }
    if h.starts_with("*.") {
        if skip_wildcards {
            return None;
        }
        return Some(h.trim_start_matches("*.").to_string());
    }
    // Basic sanity: must look like a hostname (no spaces, at least one dot).
    if h.contains(' ') || !h.contains('.') {
        return None;
    }
    Some(h)
}

/// Parse crt.sh JSON into the unique set of in-scope hostnames.
///
/// Exposed (and unit-tested) separately from the network fetch so the parsing
/// logic can be verified without hitting crt.sh.
#[must_use]
pub fn parse_crtsh_json(body: &str, domain: &str, skip_wildcards: bool) -> Vec<String> {
    let entries: Vec<CrtShEntry> = serde_json::from_str(body).unwrap_or_default();
    let suffix = format!(".{}", domain.to_ascii_lowercase());
    let domain_lc = domain.to_ascii_lowercase();

    let mut set: BTreeSet<String> = BTreeSet::new();
    for entry in entries {
        // name_value can hold multiple names separated by newlines.
        let names = entry
            .name_value
            .split([',', '\n'])
            .chain(std::iter::once(entry.common_name.as_str()));
        for raw in names {
            if let Some(host) = clean_host(raw, skip_wildcards).filter(|h| *h == domain_lc || h.ends_with(&suffix)) {
                set.insert(host);
            }
        }
    }
    set.into_iter().collect()
}

/// Fetch the CT-log hostname set for `domain` from crt.sh (no probing).
///
/// crt.sh is operationally flaky and frequently answers 502/503 under load,
/// so we retry a handful of times with linear backoff before giving up.
///
/// # Errors
/// Network or decode errors talking to crt.sh after all retries are exhausted.
pub async fn fetch_subdomains(client: &reqwest::Client, cfg: &ReconConfig) -> Result<Vec<String>, String> {
    const MAX_ATTEMPTS: u32 = 4;
    let url = format!("https://crt.sh/?q=%25.{}&output=json", cfg.domain);
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match client
            .get(&url)
            .header("User-Agent", "js-beautify-rs/domain-recon")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.map_err(|e| format!("crt.sh read body: {e}"))?;
                return Ok(parse_crtsh_json(&body, &cfg.domain, cfg.skip_wildcards));
            }
            // crt.sh emits transient 404/502/503 under load — all retryable.
            Ok(resp) => last_err = format!("crt.sh returned HTTP {}", resp.status()),
            Err(e) => last_err = format!("crt.sh request failed: {e}"),
        }
        if attempt < MAX_ATTEMPTS {
            eprintln!("[crtsh] attempt {attempt}/{MAX_ATTEMPTS} failed ({last_err}); retrying…");
            tokio::time::sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
        }
    }
    Err(format!("{last_err} (after {MAX_ATTEMPTS} attempts)"))
}

/// Extract in-scope hostnames from a Wayback Machine CDX response.
///
/// The CDX API (`web.archive.org/cdx/search/cdx?url=*.<domain>/*&output=json&fl=original`)
/// returns a JSON array of `[original_url]` rows (first row is the header). We
/// pull the host out of each archived URL and keep the in-scope ones. Exposed +
/// unit-tested separately from the fetch.
#[must_use]
pub fn parse_wayback_cdx(body: &str, domain: &str) -> Vec<String> {
    let suffix = format!(".{}", domain.to_ascii_lowercase());
    let domain_lc = domain.to_ascii_lowercase();
    let rows: Vec<Vec<String>> = serde_json::from_str(body).unwrap_or_default();
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (i, row) in rows.iter().enumerate() {
        if i == 0 {
            continue; // header row: ["original"]
        }
        let Some(url) = row.first() else { continue };
        if let Some(host) = host_from_url(url).filter(|h| *h == domain_lc || h.ends_with(&suffix)) {
            set.insert(host);
        }
    }
    set.into_iter().collect()
}

/// Pull the lowercase host from a URL string.
fn host_from_url(url: &str) -> Option<String> {
    let after = url.split("://").nth(1).unwrap_or(url);
    let host = after.split(['/', ':', '?', '@']).next()?;
    let host = host.rsplit('@').next().unwrap_or(host); // strip userinfo
    (!host.is_empty() && host.contains('.')).then(|| host.to_ascii_lowercase())
}

/// Fetch subdomains from the Wayback Machine CDX index (passive, no key).
///
/// # Errors
/// Network/decode errors talking to web.archive.org.
pub async fn fetch_wayback(client: &reqwest::Client, cfg: &ReconConfig) -> Result<Vec<String>, String> {
    let url = format!(
        "https://web.archive.org/cdx/search/cdx?url=*.{}/*&output=json&fl=original&collapse=urlkey&limit=20000",
        cfg.domain
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "js-beautify-rs/domain-recon")
        .send()
        .await
        .map_err(|e| format!("wayback request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("wayback returned HTTP {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| format!("wayback read body: {e}"))?;
    Ok(parse_wayback_cdx(&body, &cfg.domain))
}

/// Fuse + dedup hostnames from multiple sources into one sorted set.
#[must_use]
pub fn fuse_sources(sources: &[Vec<String>]) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for s in sources {
        set.extend(s.iter().cloned());
    }
    set.into_iter().collect()
}

/// Enumerate via all passive sources (crt.sh + Wayback) and fuse the results.
/// A failure in one source is non-fatal; the others still contribute.
///
/// # Errors
/// Only errors if *every* source fails.
pub async fn fetch_all_sources(client: &reqwest::Client, cfg: &ReconConfig) -> Result<Vec<String>, String> {
    let crt = fetch_subdomains(client, cfg).await;
    let way = fetch_wayback(client, cfg).await;
    match (&crt, &way) {
        (Err(e1), Err(e2)) => Err(format!("all sources failed: crt.sh: {e1}; wayback: {e2}")),
        _ => Ok(fuse_sources(&[crt.unwrap_or_default(), way.unwrap_or_default()])),
    }
}

/// Probe a single host over HTTPS, returning a [`Subdomain`] when reachable.
async fn probe_host(client: &reqwest::Client, host: &str) -> Subdomain {
    let url = format!("https://{host}/");
    // Per-request timeout overrides the long CT-fetch client budget so a dead
    // host fails fast instead of hanging for the full client timeout.
    #[allow(clippy::option_if_let_else)]
    match client.get(&url).timeout(Duration::from_secs(8)).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let header = |name: &str| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(ToString::to_string)
            };
            Subdomain {
                host: host.to_string(),
                status: Some(status),
                server: header("server"),
                content_type: header("content-type"),
                final_url: Some(resp.url().to_string()),
            }
        }
        Err(_) => Subdomain {
            host: host.to_string(),
            status: None,
            server: None,
            content_type: None,
            final_url: None,
        },
    }
}

/// Run a full recon: enumerate via crt.sh, then optionally probe each host.
///
/// # Errors
/// Propagates crt.sh fetch errors. Probe failures are recorded per-host, not
/// returned as a hard error.
pub async fn run(cfg: &ReconConfig) -> Result<ReconResult, String> {
    // CT fetch can be slow (multi-MB JSON); give it the full timeout budget.
    let crt_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let hosts = if cfg.use_wayback {
        fetch_all_sources(&crt_client, cfg).await?
    } else {
        fetch_subdomains(&crt_client, cfg).await?
    };

    let live = if cfg.probe {
        use futures_util::stream::{self, StreamExt};
        // Probes use a short per-host timeout so dead hosts don't stall the run.
        let probe_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| format!("build probe client: {e}"))?;
        let probed: Vec<Subdomain> = stream::iter(hosts.clone())
            .map(|h| {
                let client = probe_client.clone();
                async move { probe_host(&client, &h).await }
            })
            .buffer_unordered(cfg.concurrency.max(1))
            .collect()
            .await;
        // Keep only reachable hosts (got an HTTP status), sorted by host.
        let mut up: Vec<Subdomain> = probed.into_iter().filter(|s| s.status.is_some()).collect();
        up.sort_by(|a, b| a.host.cmp(&b.host));
        up
    } else {
        Vec::new()
    };

    Ok(ReconResult {
        domain: cfg.domain.clone(),
        hosts,
        live,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_scopes_crtsh_names() {
        let body = r#"[
            {"name_value":"www.blackrock.com\nblackrock.com","common_name":"www.blackrock.com"},
            {"name_value":"*.api.blackrock.com","common_name":"*.api.blackrock.com"},
            {"name_value":"login.blackrock.com","common_name":"login.blackrock.com"},
            {"name_value":"evil.example.com","common_name":"evil.example.com"}
        ]"#;
        let hosts = parse_crtsh_json(body, "blackrock.com", true);
        assert!(hosts.contains(&"www.blackrock.com".to_string()));
        assert!(hosts.contains(&"blackrock.com".to_string()));
        assert!(hosts.contains(&"login.blackrock.com".to_string()));
        // Out-of-scope domain is excluded.
        assert!(!hosts.iter().any(|h| h.contains("example.com")));
        // Wildcard dropped when skip_wildcards = true.
        assert!(!hosts.iter().any(|h| h.starts_with('*')));
        // Sorted + deduplicated.
        let mut sorted = hosts.clone();
        sorted.sort();
        assert_eq!(hosts, sorted);
    }

    #[test]
    fn keeps_wildcard_root_when_requested() {
        let body = r#"[{"name_value":"*.cdn.blackrock.com","common_name":""}]"#;
        let hosts = parse_crtsh_json(body, "blackrock.com", false);
        assert_eq!(hosts, vec!["cdn.blackrock.com".to_string()]);
    }

    #[test]
    fn empty_or_garbage_json_yields_no_hosts() {
        assert!(parse_crtsh_json("not json", "blackrock.com", true).is_empty());
        assert!(parse_crtsh_json("[]", "blackrock.com", true).is_empty());
    }

    #[test]
    fn parses_wayback_cdx_hosts_in_scope() {
        let body = r#"[["original"],
            ["https://www.blackrock.com/a.js"],
            ["http://api.blackrock.com:443/v1?x=1"],
            ["https://cdn.other.com/x.js"],
            ["https://blackrock.com/"]]"#;
        let hosts = parse_wayback_cdx(body, "blackrock.com");
        assert!(hosts.contains(&"www.blackrock.com".to_string()));
        assert!(hosts.contains(&"api.blackrock.com".to_string()));
        assert!(hosts.contains(&"blackrock.com".to_string()));
        assert!(!hosts.iter().any(|h| h.contains("other.com")));
        // Sorted + deduped (header row skipped).
        let mut s = hosts.clone();
        s.sort();
        assert_eq!(hosts, s);
    }

    #[test]
    fn wayback_garbage_is_empty() {
        assert!(parse_wayback_cdx("not json", "x.com").is_empty());
        assert!(parse_wayback_cdx("[]", "x.com").is_empty());
    }

    #[test]
    fn fuse_dedups_across_sources() {
        let a = vec!["a.x.com".to_string(), "b.x.com".to_string()];
        let b = vec!["b.x.com".to_string(), "c.x.com".to_string()];
        let fused = fuse_sources(&[a, b]);
        assert_eq!(fused, vec!["a.x.com", "b.x.com", "c.x.com"]);
    }
}
