//! HTTP Fuzzer — Burp Intruder clone.
//!
//! Ports `crates/core/jxscout-app/src/commands/fuzz.rs` from jxscout-pro-v2.
//!
//! Attack modes:
//!   - **Sniper**: one marker at a time, others stay original
//!   - **Battering ram**: same payload in all markers simultaneously
//!   - **Pitchfork**: zip across wordlists (1st word from each list together)
//!   - **Clusterbomb**: cartesian product of all wordlists

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::{fmt, fs};

/// Attack mode determines how payloads map to positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackMode {
    Sniper,
    BatteringRam,
    Pitchfork,
    Clusterbomb,
}

impl fmt::Display for AttackMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sniper => write!(f, "sniper"),
            Self::BatteringRam => write!(f, "battering-ram"),
            Self::Pitchfork => write!(f, "pitchfork"),
            Self::Clusterbomb => write!(f, "clusterbomb"),
        }
    }
}

impl std::str::FromStr for AttackMode {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().replace('-', "").as_str() {
            "sniper" => Ok(Self::Sniper),
            "batteringram" => Ok(Self::BatteringRam),
            "pitchfork" => Ok(Self::Pitchfork),
            "clusterbomb" => Ok(Self::Clusterbomb),
            _ => Err(()),
        }
    }
}

/// A parsed raw HTTP request template.
#[derive(Debug, Clone)]
pub struct RequestTemplate {
    pub method: String,
    pub path: String,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The raw text used for marker replacement.
    pub raw: String,
}

/// A single fuzz result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FuzzResult {
    pub index: usize,
    pub payloads: Vec<String>,
    pub status_code: u16,
    pub response_size: usize,
    pub body_preview: String,
}

/// Parse a raw HTTP request (Burp-format `.req` file).
///
/// # Errors
/// Returns an error when the request line is empty or malformed.
pub fn parse_request(raw: &str) -> Result<RequestTemplate, String> {
    let mut lines = raw.lines();

    // First line: METHOD PATH HTTP/VERSION
    let request_line = lines.next().ok_or("Empty request")?;
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("Invalid request line: {request_line}"));
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();
    let http_version = parts.get(2).unwrap_or(&"HTTP/1.1").to_string();

    // Headers until empty line
    let mut headers = Vec::new();
    let mut body_start = false;
    let mut body_lines = Vec::new();

    for line in lines {
        if body_start {
            body_lines.push(line);
        } else if line.is_empty() || line == "\r" {
            body_start = true;
        } else if let Some((key, val)) = line.split_once(':') {
            headers.push((key.trim().to_string(), val.trim().to_string()));
        }
    }

    Ok(RequestTemplate {
        method,
        path,
        http_version,
        headers,
        body: body_lines.join("\n"),
        raw: raw.to_string(),
    })
}

/// Find all marker positions in a template string.
/// Default markers: FUZZ, FUZZ2, FUZZ3, ...
#[must_use]
pub fn find_markers(template: &str, markers: &[String]) -> Vec<(String, Vec<usize>)> {
    markers
        .iter()
        .map(|m| {
            let positions: Vec<usize> = template.match_indices(m.as_str()).map(|(i, _)| i).collect();
            (m.clone(), positions)
        })
        .filter(|(_, positions)| !positions.is_empty())
        .collect()
}

/// Load a wordlist from a file (one word per line).
///
/// # Errors
/// Returns an error when the file cannot be opened or a line cannot be read.
pub fn load_wordlist(path: &Path) -> Result<Vec<String>, String> {
    let file = fs::File::open(path).map_err(|e| format!("Cannot open wordlist {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    Ok(reader
        .lines()
        .map_while(std::result::Result::ok)
        .filter(|l| !l.is_empty())
        .collect())
}

/// Generate all payload combinations for the given attack mode.
#[must_use]
pub fn generate_combinations(wordlists: &[Vec<String>], mode: AttackMode) -> Vec<Vec<String>> {
    if wordlists.is_empty() {
        return Vec::new();
    }

    match mode {
        AttackMode::Sniper => {
            // One position at a time. Others get their original value (empty string).
            let mut combos = Vec::new();
            for (pos, wl) in wordlists.iter().enumerate() {
                for payload in wl {
                    let mut combo: Vec<String> = wordlists.iter().map(|_| String::new()).collect();
                    payload.clone_into(&mut combo[pos]);
                    combos.push(combo);
                }
            }
            combos
        }
        AttackMode::BatteringRam => {
            // Same payload in all positions (use first wordlist).
            let wl = &wordlists[0];
            wl.iter()
                .map(|payload| vec![payload.clone(); wordlists.len()])
                .collect()
        }
        AttackMode::Pitchfork => {
            // Zip across wordlists.
            let min_len = wordlists.iter().map(std::vec::Vec::len).min().unwrap_or(0);
            (0..min_len)
                .map(|i| wordlists.iter().map(|wl| wl[i].clone()).collect())
                .collect()
        }
        AttackMode::Clusterbomb => {
            // Cartesian product.
            let mut combos: Vec<Vec<String>> = vec![Vec::new()];
            for wl in wordlists {
                let mut new_combos = Vec::new();
                for existing in &combos {
                    for payload in wl {
                        let mut combo = existing.clone();
                        combo.push(payload.clone());
                        new_combos.push(combo);
                    }
                }
                combos = new_combos;
            }
            combos
        }
    }
}

/// Apply a set of payloads to a template, replacing markers in order.
#[must_use]
pub fn apply_payloads(template: &str, markers: &[String], payloads: &[String]) -> String {
    let mut result = template.to_string();
    for (i, marker) in markers.iter().enumerate() {
        let payload = payloads.get(i).map_or("", std::string::String::as_str);
        result = result.replacen(marker.as_str(), payload, 1);
    }
    result
}

/// Derive the Host and port from the request headers.
#[must_use]
pub fn extract_host(template: &RequestTemplate) -> Option<(String, u16, bool)> {
    for (k, v) in &template.headers {
        if k.eq_ignore_ascii_case("Host") {
            let host = v.trim();
            if let Some((h, p)) = host.rsplit_once(':') {
                let port: u16 = p.parse().unwrap_or(443);
                return Some((h.to_string(), port, port == 443));
            }
            return Some((host.to_string(), 443, true));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_basic() {
        let raw = "GET /api/users?id=FUZZ HTTP/1.1\r\nHost: example.com\r\nCookie: session=abc\r\n\r\n";
        let tmpl = parse_request(raw).unwrap();
        assert_eq!(tmpl.method, "GET");
        assert_eq!(tmpl.path, "/api/users?id=FUZZ");
        assert_eq!(tmpl.headers.len(), 2);
    }

    #[test]
    fn sniper_combinations() {
        let wl = vec![vec!["a".into(), "b".into()], vec!["1".into(), "2".into()]];
        let combos = generate_combinations(&wl, AttackMode::Sniper);
        // 2 from first position + 2 from second = 4
        assert_eq!(combos.len(), 4);
    }

    #[test]
    fn clusterbomb_combinations() {
        let wl = vec![vec!["a".into(), "b".into()], vec!["1".into(), "2".into()]];
        let combos = generate_combinations(&wl, AttackMode::Clusterbomb);
        // 2 * 2 = 4
        assert_eq!(combos.len(), 4);
        assert!(combos.contains(&vec!["a".to_string(), "1".to_string()]));
        assert!(combos.contains(&vec!["b".to_string(), "2".to_string()]));
    }

    #[test]
    fn pitchfork_combinations() {
        let wl = vec![vec!["a".into(), "b".into(), "c".into()], vec!["1".into(), "2".into()]];
        let combos = generate_combinations(&wl, AttackMode::Pitchfork);
        assert_eq!(combos.len(), 2); // min length
        assert_eq!(combos[0], vec!["a".to_string(), "1".to_string()]);
    }

    #[test]
    fn apply_payloads_replaces_markers() {
        let template = "GET /api/users/FUZZ?role=FUZZ2 HTTP/1.1";
        let markers = vec!["FUZZ".into(), "FUZZ2".into()];
        let result = apply_payloads(template, &markers, &["123".into(), "admin".into()]);
        assert_eq!(result, "GET /api/users/123?role=admin HTTP/1.1");
    }
}
