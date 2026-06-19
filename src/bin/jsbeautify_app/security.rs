pub(super) fn run_vuln_scan(
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
    file_path: Option<&str>,
) -> Vec<js_beautify_rs::analyzers::AnalyzerMatch> {
    use js_beautify_rs::analyzers::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
    use js_beautify_rs::analyzers::regex_bank::Severity;
    use js_beautify_rs::vuln_db::{self, Component};

    let mut components = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |name: &str, version: &str, comps: &mut Vec<Component>| {
        let name = name.trim();
        let version = version.trim();
        if name.is_empty() || version.is_empty() || !version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return;
        }
        if seen.insert((name.to_string(), version.to_string())) {
            comps.push(Component::npm(name, version));
        }
    };
    for m in matches {
        let meta = m.metadata.as_ref();
        let mname = meta.and_then(|v| v.get("name")).and_then(|v| v.as_str());
        let mver = meta.and_then(|v| v.get("version")).and_then(|v| v.as_str());
        match (mname, mver) {
            (Some(n), Some(v)) => add(n, v, &mut components),
            _ => {
                if let Some((name, version)) = m.match_value.rsplit_once('@') {
                    add(name, version, &mut components);
                }
            }
        }
    }
    if components.is_empty() {
        eprintln!("[vuln-scan] no versioned npm packages detected; nothing to look up");
        return Vec::new();
    }
    eprintln!(
        "[vuln-scan] querying OSV + CISA KEV for {} component(s)…",
        components.len()
    );

    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else {
        return Vec::new();
    };
    let vulns = match runtime.block_on(vuln_db::scan(&components)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[vuln-scan] lookup failed: {e}");
            return Vec::new();
        }
    };

    vulns
        .into_iter()
        .map(|v| {
            let severity = if v.kev {
                Severity::Critical
            } else {
                match v.cvss {
                    Some(s) if s >= 9.0 => Severity::Critical,
                    Some(s) if s >= 7.0 => Severity::High,
                    Some(s) if s >= 4.0 => Severity::Medium,
                    Some(_) => Severity::Low,
                    None => Severity::High,
                }
            };
            AnalyzerMatch::new(
                "known_cve",
                format!("{}@{} {}", v.component, v.version, v.id),
                PositionRange::new(1, 0, 1, 1),
                FileType::Js,
                file_path.map(String::from),
            )
            .with_provenance(Provenance::Derived)
            .with_severity(severity)
            .with_metadata(serde_json::json!({
                "cve": v.id,
                "component": v.component,
                "version": v.version,
                "aliases": v.aliases,
                "summary": v.summary,
                "kev": v.kev,
                "cvss": v.cvss,
                "cvss_vector": v.cvss_vector,
                "epss": v.epss,
            }))
        })
        .collect()
}

pub(super) fn run_agent(
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
    source: &str,
) -> Option<js_beautify_rs::agent::AgentReport> {
    use js_beautify_rs::agent::{Investigation, investigate};
    use js_beautify_rs::llm::{LlmClient, LlmOverrides};

    let Some(mut client) = LlmClient::from_overrides(&LlmOverrides::default()) else {
        eprintln!("[agent] no LLM endpoint configured (set OPENAI_API_KEY or --llm-url); skipping agent loop.");
        return None;
    };
    let investigation = Investigation::new(matches, Some(source));
    match investigate(&mut client, &investigation, 12) {
        Ok(report) => Some(report),
        Err(e) => {
            eprintln!("[agent] investigation failed: {e}");
            None
        }
    }
}

#[derive(serde::Serialize)]
pub(super) struct ValidationRow {
    pub(super) target: String,
    pub(super) verdict_str: String,
    pub(super) evidence: String,
    pub(super) finding_kind: String,
}

pub(super) fn run_validation(
    validate_scope: Option<&str>,
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
) -> Vec<ValidationRow> {
    use js_beautify_rs::validate::{LiveProber, ProbeConfig, Verdict};

    let Some(scope_raw) = validate_scope else {
        eprintln!("[validate] --validate requires --validate-scope (in-scope host allowlist); skipping.");
        return Vec::new();
    };
    let patterns: Vec<&str> = scope_raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if patterns.is_empty() {
        eprintln!("[validate] empty --validate-scope; skipping.");
        return Vec::new();
    }

    let prober = match LiveProber::new(&patterns, ProbeConfig::default()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[validate] could not build prober: {e}");
            return Vec::new();
        }
    };

    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else {
        return Vec::new();
    };

    let verdict_str = |v: &Verdict| match v {
        Verdict::Confirmed => "CONFIRMED",
        Verdict::Refuted => "refuted",
        Verdict::Inconclusive => "inconclusive",
    };

    let mut rows = Vec::new();
    runtime.block_on(async {
        for m in matches {
            let base = m.file_path.as_deref().unwrap_or("");
            let url = resolve_probe_url(&m.match_value, base);
            let Some(url) = url else { continue };
            if !prober.in_scope(&url) {
                continue;
            }
            let validation = match m.match_kind.as_str() {
                "open_redirect" => prober.probe_open_redirect(&url).await,
                "dom_xss" => prober.probe_reflection(&url, "q").await,
                "idor_candidate" => {
                    let Some(swapped) = js_beautify_rs::validate::swap_first_id(&url) else {
                        continue;
                    };
                    prober.probe_idor(&url, &swapped).await
                }
                "known_cve" => {
                    let version = m
                        .metadata
                        .as_ref()
                        .and_then(|v| v.get("version"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    prober.probe_known_cve(&url, version).await
                }
                _ => continue,
            };
            rows.push(ValidationRow {
                target: url,
                verdict_str: verdict_str(&validation.verdict).to_string(),
                evidence: validation.evidence,
                finding_kind: m.match_kind.clone(),
            });
        }
    });
    rows
}

fn resolve_probe_url(value: &str, base: &str) -> Option<String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Some(value.to_string());
    }
    if value.starts_with('/') {
        let scheme_end = base.find("://")?;
        let after = &base[scheme_end + 3..];
        let host_end = after.find('/').unwrap_or(after.len());
        return Some(format!("{}://{}{}", &base[..scheme_end], &after[..host_end], value));
    }
    None
}
