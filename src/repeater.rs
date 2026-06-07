//! HTTP Request Repeater — replay raw .req files.
//!
//! Ports `crates/core/jxscout-app/src/commands/repeater.rs` from jxscout-pro-v2.
//!
//! Parses Burp-exported `.req` files, sends them over HTTP(S), and saves
//! the response as a `.res` file alongside.

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::fuzz;

/// Configuration for the repeater.
#[derive(Debug, Clone)]
pub struct RepeaterConfig {
    /// Override target URL (e.g. "https://host:443"). If None, derived from Host header.
    pub url: Option<String>,
    /// Proxy host (optional).
    pub proxy_host: Option<String>,
    /// Proxy port (optional).
    pub proxy_port: Option<u16>,
    /// Auto-update Content-Length header.
    pub update_content_length: bool,
    /// Timeout for the request in seconds.
    pub timeout_secs: u64,
}

impl Default for RepeaterConfig {
    fn default() -> Self {
        Self {
            url: None,
            proxy_host: None,
            proxy_port: None,
            update_content_length: true,
            timeout_secs: 30,
        }
    }
}

/// Result of a repeated request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RepeaterResult {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub body_size: usize,
    pub elapsed_ms: u128,
}

/// Replay a raw .req file and return the response.
pub fn replay(req_content: &str, config: &RepeaterConfig) -> Result<RepeaterResult, String> {
    let mut template = fuzz::parse_request(req_content)?;

    // Update Content-Length if needed
    if config.update_content_length && !template.body.is_empty() {
        let body_len = template.body.len();
        let mut found = false;
        for (k, v) in &mut template.headers {
            if k.eq_ignore_ascii_case("Content-Length") {
                *v = body_len.to_string();
                found = true;
                break;
            }
        }
        if !found {
            template
                .headers
                .push(("Content-Length".to_string(), body_len.to_string()));
        }
    }

    // Determine target URL
    let target_url = if let Some(ref url) = config.url {
        format!("{}{}", url.trim_end_matches('/'), &template.path)
    } else {
        // Derive from Host header
        let (host, port, is_tls) = fuzz::extract_host(&template).ok_or("No Host header and no --url specified")?;
        let scheme = if is_tls { "https" } else { "http" };
        if (is_tls && port == 443) || (!is_tls && port == 80) {
            format!("{scheme}://{host}{}", template.path)
        } else {
            format!("{scheme}://{host}:{port}{}", template.path)
        }
    };

    // Build reqwest client
    let mut client_builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .danger_accept_invalid_certs(true);

    if let (Some(ph), Some(pp)) = (&config.proxy_host, config.proxy_port) {
        let proxy_url = format!("http://{}:{}", ph, pp);
        client_builder =
            client_builder.proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| format!("Invalid proxy: {e}"))?);
    }

    let client = client_builder.build().map_err(|e| format!("Client build error: {e}"))?;

    // Build request
    let method = reqwest::Method::from_bytes(template.method.as_bytes())
        .map_err(|e| format!("Invalid method '{}': {e}", template.method))?;

    let mut req = client.request(method, &target_url);

    for (k, v) in &template.headers {
        // Skip Host (reqwest sets it) and Connection
        if k.eq_ignore_ascii_case("Host") || k.eq_ignore_ascii_case("Connection") {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }

    if !template.body.is_empty() {
        req = req.body(template.body.clone());
    }

    let start = std::time::Instant::now();
    let response = req.send().map_err(|e| format!("Request failed: {e}"))?;
    let elapsed_ms = start.elapsed().as_millis();

    let status_code = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = response.text().map_err(|e| format!("Failed to read body: {e}"))?;
    let body_size = body.len();

    Ok(RepeaterResult {
        status_code,
        headers,
        body,
        body_size,
        elapsed_ms,
    })
}

/// Replay a .req file from disk, save the response as .res alongside.
pub fn replay_file(req_path: &Path, config: &RepeaterConfig) -> Result<RepeaterResult, String> {
    let content = fs::read_to_string(req_path).map_err(|e| format!("Cannot read {}: {e}", req_path.display()))?;

    let result = replay(&content, config)?;

    // Save .res file
    let res_path = req_path.with_extension("res");
    let res_content = format_response(&result);
    fs::write(&res_path, &res_content).map_err(|e| format!("Cannot write {}: {e}", res_path.display()))?;

    Ok(result)
}

/// Format a response in raw HTTP format.
fn format_response(result: &RepeaterResult) -> String {
    let mut out = format!("HTTP/1.1 {}\r\n", result.status_code);
    for (k, v) in &result.headers {
        out.push_str(&format!("{}: {}\r\n", k, v));
    }
    out.push_str("\r\n");
    out.push_str(&result.body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_response_looks_correct() {
        let result = RepeaterResult {
            status_code: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: r#"{"ok":true}"#.into(),
            body_size: 11,
            elapsed_ms: 42,
        };
        let formatted = format_response(&result);
        assert!(formatted.starts_with("HTTP/1.1 200\r\n"));
        assert!(formatted.contains("content-type: application/json"));
        assert!(formatted.ends_with(r#"{"ok":true}"#));
    }
}
