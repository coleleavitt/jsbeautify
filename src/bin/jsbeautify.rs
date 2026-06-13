// Heap profiling: with `--features dhat-heap`, dhat replaces the global
// allocator and records every allocation's call site (see Cargo.toml).
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use clap::Parser;
use js_beautify_rs::analyzers::match_kind::FileType;
use js_beautify_rs::analyzers::{AnalyzerConfig, run_all};
use js_beautify_rs::cross_version::{AlignConfig, CrossVersionAligner};
use js_beautify_rs::tokenizer::Tokenizer;
use js_beautify_rs::webpack_module_extractor::ModuleExtractor;
use js_beautify_rs::{Options, beautify};
use regex;
use serde_json;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

/// A fast JavaScript beautifier and deobfuscator powered by oxc.
///
/// Beautifies minified JavaScript, deobfuscates obfuscated webpack bundles,
/// and extracts webpack modules into separate files.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[command(name = "jsbeautify", version, about, long_about = None)]
struct Cli {
    /// Input JavaScript file, directory (with --dir), or "-" for stdin.
    /// Optional in --serve mode.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Write output to a file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Recursively scan a directory (analyze all .js/.html/.json files)
    #[arg(long)]
    dir: bool,

    /// Glob patterns to exclude when scanning directories (repeatable)
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Enable AST-based deobfuscation (19-phase pipeline)
    #[arg(short, long)]
    deobfuscate: bool,

    /// Infer readable names for minified locals (loop→i/j/k, catch→error,
    /// require→module, new Foo→instance) instead of v{n} slots. Implies
    /// --deobfuscate.
    #[arg(long)]
    rename_vars: bool,

    /// After heuristic renaming, ask an LLM to name residual minified locals
    /// (humanify pattern). Works with a standard OPENAI_API_KEY, or any
    /// OpenAI-compatible endpoint via --llm-url / env. Implies --rename-vars.
    #[arg(long)]
    rename_llm: bool,

    /// OpenAI-compatible endpoint URL (overrides OPENAI_BASE_URL /
    /// JSBEAUTIFY_LLM_URL). A bare base gets `/chat/completions` appended.
    #[arg(long, value_name = "URL", global = true)]
    llm_url: Option<String>,

    /// LLM model id (overrides OPENAI_MODEL / JSBEAUTIFY_LLM_MODEL).
    #[arg(long, value_name = "MODEL", global = true)]
    llm_model: Option<String>,

    /// LLM API key (overrides OPENAI_API_KEY / JSBEAUTIFY_LLM_KEY).
    #[arg(long, value_name = "KEY", global = true)]
    llm_key: Option<String>,

    /// Split webpack chunks into separate files
    #[arg(long)]
    split_chunks: bool,

    /// Directory for chunk output [default: ./chunks]
    #[arg(long, value_name = "DIR")]
    chunk_dir: Option<PathBuf>,

    /// Write chunk metadata to a JSON file
    #[arg(long, value_name = "FILE")]
    chunk_map: Option<PathBuf>,

    /// Extract webpack modules to separate files
    #[arg(long)]
    extract_modules: bool,

    /// Directory for module output [default: ./modules]
    #[arg(long, value_name = "DIR")]
    module_dir: Option<PathBuf>,

    /// Generate a dependency graph in DOT format
    #[arg(long, value_name = "FILE")]
    dependency_graph: Option<PathBuf>,

    /// Generate source maps
    #[arg(long)]
    source_maps: bool,

    /// Indentation size in spaces [default: 4]
    #[arg(long, value_name = "N")]
    indent_size: Option<usize>,

    /// Use tabs for indentation instead of spaces
    #[arg(long)]
    indent_with_tabs: bool,

    /// Path to sourcemap for extracting original variable names
    #[arg(long, value_name = "FILE")]
    sourcemap: Option<PathBuf>,

    /// Path to names.json with full name mappings (from extract-names.ts)
    #[arg(long, value_name = "FILE")]
    names_json: Option<PathBuf>,

    /// Second bundle to align with (produces stable diffs)
    #[arg(long, value_name = "FILE")]
    align_with: Option<PathBuf>,

    /// Output path for the aligned second bundle
    #[arg(long, value_name = "FILE")]
    align_output: Option<PathBuf>,

    /// Skip beautification and output raw aligned code
    #[arg(long)]
    raw: bool,

    /// Extract names from Bun bundle patterns (MR exports, this.name, displayName)
    #[arg(long)]
    bun_extract: bool,

    // ── Security analysis ────────────────────────────────────────────────────
    /// Run all security analyzers (secrets, sinks, paths, GraphQL, npm packages)
    #[arg(long)]
    analyze: bool,

    /// Scan for secrets and credentials only (fast, regex-based)
    #[arg(long)]
    scan_secrets: bool,

    /// Detect dangerous sinks (innerHTML, postMessage, location.href, …)
    #[arg(long)]
    find_sinks: bool,

    /// Extract paths, hostnames, and URLs
    #[arg(long)]
    extract_paths: bool,

    /// Extract only hostnames (jxscout-compatible separate analyzer)
    #[arg(long)]
    extract_hostnames: bool,

    /// Extract GraphQL operations
    #[arg(long)]
    extract_graphql: bool,

    /// Extract npm package references
    #[arg(long)]
    extract_packages: bool,

    /// With --analyze: look up detected npm packages against the OSV + CISA KEV
    /// vulnerability catalogs and emit known_cve findings (needs network).
    #[arg(long)]
    vuln_scan: bool,

    /// With --analyze: correlate findings into multi-step attack chains and
    /// include them in the report.
    #[arg(long)]
    chains: bool,

    /// With --analyze: run the agentic LLM investigation loop over the findings
    /// (chains them into attack paths). Needs an LLM endpoint (see --llm-url /
    /// OPENAI_API_KEY).
    #[arg(long)]
    agent: bool,

    /// With --analyze: actively confirm findings against the live target by
    /// probing (open-redirect, reflection, IDOR, known-CVE). ONLY hosts matching
    /// --validate-scope are ever contacted. Requires authorization for the target.
    #[arg(long)]
    validate: bool,

    /// Comma-separated in-scope host patterns for --validate (e.g.
    /// "*.target.com,target.com"). Nothing outside this allowlist is requested.
    #[arg(long, value_name = "PATTERNS")]
    validate_scope: Option<String>,

    /// Detect sensitive property names (secret, authorization, aws_secret_key, …)
    #[arg(long)]
    find_sensitive_props: bool,

    /// Generate a fuzzing wordlist from JS identifiers and string literals
    #[arg(long)]
    wordlist: bool,

    /// Minimum word length for wordlist output [default: 2]
    #[arg(long, value_name = "N")]
    min_length: Option<usize>,

    /// Maximum word length for wordlist output (0 = unlimited) [default: 0]
    #[arg(long, value_name = "N")]
    max_length: Option<usize>,

    /// Convert all wordlist entries to lowercase
    #[arg(long)]
    lowercase: bool,

    /// Generate plural/singular forms for each word
    #[arg(long)]
    plurals: bool,

    /// Filter wordlist by extraction source (e.g. js_variable_name, html_body_text). Repeatable.
    #[arg(long, value_name = "SOURCE")]
    sources: Vec<String>,

    /// Sort wordlist by occurrence count (default: alphabetical)
    #[arg(long)]
    sort_by_count: bool,

    /// Maximum number of words to output (0 = unlimited)
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Custom regex pattern to scan for (repeatable). Format: "label:pattern"
    #[arg(long, value_name = "LABEL:REGEX")]
    custom_regex: Vec<String>,

    /// Analyze HTML file (extract <script> blocks and run analyzers on them)
    #[arg(long)]
    analyze_html: bool,

    /// Discover Next.js chunks from page HTML (outputs chunk URLs)
    #[arg(long)]
    discover_nextjs: bool,

    // ── Whole-domain recon (crt.sh) ──────────────────────────────────────────
    /// Enumerate every subdomain of a registrable domain via Certificate
    /// Transparency logs (crt.sh), so you can mine the whole domain instead of
    /// just one captured path. Value is the domain, e.g. "blackrock.com".
    #[arg(long, value_name = "DOMAIN")]
    crtsh: Option<String>,

    /// With --crtsh: send an HTTPS request to each discovered host and report
    /// which are live (status, Server header, final URL after redirects).
    #[arg(long)]
    probe: bool,

    /// With --crtsh / --recon: also enumerate subdomains via the Wayback Machine
    /// CDX index and fuse with crt.sh results.
    #[arg(long)]
    wayback: bool,

    /// Recursive recon: crt.sh subdomains → fetch JS/HTML/source-maps from live
    /// hosts → deobfuscate → analyze → extract NEW hosts/endpoints/secrets and
    /// loop. Value is the seed domain, e.g. "blackrock.com". Use --json for the
    /// full report. Honors --recon-rounds / --recon-max-assets.
    #[arg(long, value_name = "DOMAIN")]
    recon: Option<String>,

    /// With --recon: number of recursive expansion rounds (default 2).
    #[arg(long, value_name = "N", default_value_t = 2)]
    recon_rounds: usize,

    /// With --recon: cap on total assets fetched across the whole run.
    #[arg(long, value_name = "N", default_value_t = 300)]
    recon_max_assets: usize,

    /// Base URL for Next.js chunk discovery (required with --discover-nextjs)
    #[arg(long, value_name = "URL")]
    base_url: Option<String>,

    /// Output analysis results as JSON (default: human-readable)
    #[arg(long)]
    json: bool,

    /// Write analysis/wordlist output to FILE instead of stdout
    #[arg(long, value_name = "FILE")]
    analysis_out: Option<PathBuf>,

    // ── Repeater ─────────────────────────────────────────────────────────────
    /// Replay a raw .req file (Burp export format) and print the response
    #[arg(long)]
    repeat: bool,

    /// Override target URL for repeater (e.g. "https://host:443")
    #[arg(long, value_name = "URL")]
    repeat_url: Option<String>,

    /// Proxy host for repeater/fuzz requests
    #[arg(long, value_name = "HOST")]
    proxy_host: Option<String>,

    /// Proxy port for repeater/fuzz requests
    #[arg(long, value_name = "PORT")]
    proxy_port: Option<u16>,

    // ── Fuzz ─────────────────────────────────────────────────────────────────
    /// Fuzz a .req template file with wordlists/payloads
    #[arg(long)]
    fuzz: bool,

    /// Wordlist file(s) for fuzzing (repeatable, paired with markers)
    #[arg(long, value_name = "FILE")]
    fuzz_wordlist: Vec<PathBuf>,

    /// Marker strings to replace (default: FUZZ, FUZZ2, ...). Repeatable.
    #[arg(long, value_name = "MARKER")]
    marker: Vec<String>,

    /// Inline payloads (comma-separated) for quick fuzz
    #[arg(long, value_name = "a,b,c")]
    payloads: Option<String>,

    /// Attack mode: sniper, battering-ram, pitchfork, clusterbomb [default: sniper]
    #[arg(long, value_name = "MODE")]
    attack_mode: Option<String>,

    /// Only show responses with these status codes (comma-separated)
    #[arg(long, value_name = "200,302")]
    match_status: Option<String>,

    /// Hide responses whose body contains this string
    #[arg(long, value_name = "STRING")]
    filter_body: Option<String>,

    /// Number of concurrent fuzz requests [default: 10]
    #[arg(long, value_name = "N")]
    concurrency: Option<usize>,

    /// Delay between fuzz requests in milliseconds
    #[arg(long, value_name = "MS")]
    delay_ms: Option<u64>,

    // ── Diff ─────────────────────────────────────────────────────────────────
    /// Diff two JS/HTML files — show added/removed findings
    #[arg(long, value_name = "OLD_FILE")]
    diff_with: Option<PathBuf>,

    // ── Scope ────────────────────────────────────────────────────────────────
    /// In-scope patterns (comma-separated hostnames/globs) for filtering results
    #[arg(long, value_name = "PATTERNS")]
    in_scope: Option<String>,

    /// Out-of-scope patterns (comma-separated) — exclusions
    #[arg(long, value_name = "PATTERNS")]
    out_of_scope: Option<String>,

    /// Derived analyzer: "new_kind:original_kind:include_regex" (repeatable)
    #[arg(long, value_name = "SPEC")]
    derive: Vec<String>,

    /// Read scope patterns from a file (one pattern per line)
    #[arg(long, value_name = "FILE")]
    scope_file: Option<PathBuf>,

    /// Output in SARIF format (Static Analysis Results Interchange Format)
    #[arg(long)]
    sarif: bool,

    /// Generate wordlist from analyzer match values (paths, URLs, hostnames)
    #[arg(long)]
    wordlist_from_matches: bool,

    /// Minimum severity to report (info, low, medium, high, critical)
    #[arg(long, value_name = "LEVEL")]
    min_severity: Option<String>,

    /// Deduplicate matches by (match_kind, match_value, file_path, metadata).
    /// Matches jxscout's `finding_per_unique_value` setting.
    #[arg(long)]
    unique: bool,

    /// Run all security analyzers (alias of --analyze, jxscout-compatible name).
    #[arg(long)]
    analyze_all: bool,

    /// Reverse a sourcemap (.map) file: extract every embedded `sourcesContent`
    /// entry, run analyzers against it as `FileType::ReversedSource`. Outputs
    /// per-source JSON to --analysis-out (or stdout) plus optional disk dump.
    #[arg(long, value_name = "MAP_FILE")]
    reverse_sourcemap: Option<PathBuf>,

    /// Directory to write reversed source files into (one file per source).
    /// Use with --reverse-sourcemap. Defaults to skipping disk writes.
    #[arg(long, value_name = "DIR")]
    reversed_sources_dir: Option<PathBuf>,

    /// Build the asset-relationship graph for an HTML page (jxscout's
    /// js_loader / iframe_loader / lazy-chunk graph). Use --base-url to
    /// resolve relative paths to absolute URLs.
    #[arg(long)]
    relationships: bool,

    // ── Serve daemon ─────────────────────────────────────────────────────────
    /// Run the ingestion daemon: HTTP API (:3333) + WebSocket (:9222) + MITM
    /// proxy (:8080), all feeding the beautify/deobfuscate/analyze pipeline.
    /// This is the backend the Mindemon Firefox extension POSTs captured
    /// browser assets to. When set, FILE is ignored.
    #[arg(long)]
    serve: bool,

    /// Project name for the serve daemon (DB + files live under
    /// ~/.jsbeautify/projects/<name>/).
    #[arg(long, value_name = "NAME", default_value = "default")]
    project_name: String,

    /// HTTP ingestion API port for --serve.
    #[arg(long, value_name = "PORT", default_value_t = 3333)]
    http_port: u16,

    /// WebSocket result-stream port for --serve.
    #[arg(long, value_name = "PORT", default_value_t = 9222)]
    ws_port: u16,

    /// MITM proxy port for --serve.
    #[arg(long, value_name = "PORT", default_value_t = 8080)]
    serve_proxy_port: u16,

    /// Enable TLS interception in the --serve proxy (generates/uses a CA).
    #[arg(long)]
    intercept_tls: bool,

    /// Discord webhook URL for --serve monitoring notifications.
    #[arg(long, value_name = "URL")]
    discord_webhook: Option<String>,

    /// Monitoring cycle interval in seconds for --serve (0 disables).
    #[arg(long, value_name = "SECS", default_value_t = 0)]
    monitor_interval: u64,
}

/// Build a tokio runtime and run the ingestion daemon until Ctrl-C.
fn run_serve(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    use js_beautify_rs::server::ServeConfig;

    let scope = cli
        .in_scope
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let out_of_scope = cli
        .out_of_scope
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let cfg = ServeConfig {
        project_name: cli.project_name.clone(),
        http_bind: format!("127.0.0.1:{}", cli.http_port).parse()?,
        ws_bind: format!("127.0.0.1:{}", cli.ws_port).parse()?,
        proxy_bind: format!("127.0.0.1:{}", cli.serve_proxy_port).parse()?,
        intercept_tls: cli.intercept_tls,
        scope,
        out_of_scope,
        discord_webhook: cli.discord_webhook.clone(),
        monitor_interval_secs: cli.monitor_interval,
        deobfuscate: true,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime
        .block_on(js_beautify_rs::server::serve(cfg))
        .map_err(|e| e.into())
}

/// Enumerate a domain's subdomains via crt.sh (optionally probing each), then
/// print them (JSON or plain). Builds its own tokio runtime since recon is
/// network-bound and async.
fn run_crtsh(cli: &Cli, domain: &str) -> Result<(), Box<dyn std::error::Error>> {
    use js_beautify_rs::domain_recon::{self, ReconConfig};

    let cfg = ReconConfig {
        domain: domain.to_string(),
        probe: cli.probe,
        use_wayback: cli.wayback,
        ..ReconConfig::default()
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let result = runtime
        .block_on(domain_recon::run(&cfg))
        .map_err(|e| -> Box<dyn std::error::Error> { format!("crtsh recon failed: {e}").into() })?;

    let output_text = if cli.json {
        serde_json::to_string_pretty(&result)?
    } else if cli.probe {
        let mut lines = vec![format!(
            "# {} — {} host(s) from crt.sh, {} live",
            result.domain,
            result.hosts.len(),
            result.live.len()
        )];
        for s in &result.live {
            let status = s.status.map_or_else(|| "---".to_string(), |c| c.to_string());
            let server = s.server.as_deref().unwrap_or("");
            lines.push(format!("{status:>3}  {}  {server}", s.host));
        }
        lines.join("\n")
    } else {
        result.hosts.join("\n")
    };

    if let Some(out_path) = &cli.analysis_out {
        fs::write(out_path, &output_text)?;
        eprintln!(
            "Wrote {} host(s) for {} to {}",
            result.hosts.len(),
            result.domain,
            out_path.display()
        );
    } else {
        println!("{output_text}");
    }
    Ok(())
}

/// Run the recursive recon engine: crt.sh → fetch JS/HTML/source-maps →
/// deobfuscate → analyze → harvest new hosts/endpoints/secrets → loop.
fn run_recon(cli: &Cli, domain: &str) -> Result<(), Box<dyn std::error::Error>> {
    use js_beautify_rs::recon_engine::{EngineConfig, ReconEngine};

    let engine = ReconEngine::new(EngineConfig {
        domain: domain.to_string(),
        max_rounds: cli.recon_rounds,
        max_assets: cli.recon_max_assets,
        deobfuscate: true,
        ..EngineConfig::default()
    })
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let report = runtime
        .block_on(engine.run())
        .map_err(|e| -> Box<dyn std::error::Error> { format!("recon failed: {e}").into() })?;

    let text = if cli.json {
        serde_json::to_string_pretty(&report)?
    } else {
        let mut lines = vec![
            format!("# Recursive recon — {}", report.domain),
            format!(
                "rounds={}  hosts={}  live={}  assets={}  findings={}",
                report.rounds_run,
                report.hosts.len(),
                report.live_hosts.len(),
                report.assets.len(),
                report.total_findings
            ),
            String::new(),
            format!("## Endpoints ({})", report.endpoints.len()),
        ];
        lines.extend(report.endpoints.iter().take(200).cloned());
        lines.push(String::new());
        lines.push(format!("## Secrets ({})", report.secrets.len()));
        lines.extend(report.secrets.iter().cloned());
        lines.push(String::new());
        lines.push(format!("## Live hosts ({})", report.live_hosts.len()));
        lines.extend(report.live_hosts.iter().cloned());
        lines.join("\n")
    };

    if let Some(out_path) = &cli.analysis_out {
        fs::write(out_path, &text)?;
        eprintln!(
            "Recon: {} hosts, {} live, {} assets, {} findings → {}",
            report.hosts.len(),
            report.live_hosts.len(),
            report.assets.len(),
            report.total_findings,
            out_path.display()
        );
    } else {
        println!("{text}");
    }
    Ok(())
}

/// Map `npm_package` findings → known-CVE findings via OSV + CISA KEV.
///
/// A package finding's value is `name` or `name@version`; only versioned ones
/// can be matched. Runs its own tokio runtime since the lookup is async.
fn run_vuln_scan(
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
    file_path: Option<&str>,
) -> Vec<js_beautify_rs::analyzers::AnalyzerMatch> {
    use js_beautify_rs::analyzers::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
    use js_beautify_rs::analyzers::regex_bank::Severity;
    use js_beautify_rs::vuln_db::{self, Component};

    // Collect versioned components from findings that carry name + version:
    //  - `package_json_dep` (metadata.name / metadata.version),
    //  - framework findings whose metadata has a `version`,
    //  - any `name@version`-shaped match value.
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
            // Otherwise try to parse the value as `name@version`.
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
            let severity = if v.kev { Severity::Critical } else { Severity::High };
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
            }))
        })
        .collect()
}

/// Run the agentic LLM investigation loop over the findings. Returns `None` when
/// no LLM endpoint is configured (the loop needs a real backend).
fn run_agent(
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
    source: &str,
) -> Option<js_beautify_rs::agent::AgentReport> {
    use js_beautify_rs::agent::{Investigation, investigate};
    use js_beautify_rs::llm::{LlmClient, LlmOverrides};

    let mut client = match LlmClient::from_overrides(&LlmOverrides::default()) {
        Some(c) => c,
        None => {
            eprintln!("[agent] no LLM endpoint configured (set OPENAI_API_KEY or --llm-url); skipping agent loop.");
            return None;
        }
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

/// A single live-validation result row for output.
#[derive(serde::Serialize)]
struct ValidationRow {
    target: String,
    verdict_str: String,
    evidence: String,
    finding_kind: String,
}

/// Actively confirm findings against the live (scoped) target. Only findings
/// whose value/file_path host matches `--validate-scope` are probed; everything
/// else is skipped without a request. Requires authorization for the target.
fn run_validation(
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

    // Build probe targets from findings whose value is a URL/endpoint on an
    // in-scope host. The finding's file_path (the asset URL) supplies the origin.
    let mut rows = Vec::new();
    runtime.block_on(async {
        for m in matches {
            let base = m.file_path.as_deref().unwrap_or("");
            // Resolve a probeable URL: absolute value, or endpoint joined to the asset origin.
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

/// Resolve a finding value into a probeable absolute URL, using `base` (the
/// asset URL) for origin when the value is a relative path.
fn resolve_probe_url(value: &str, base: &str) -> Option<String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Some(value.to_string());
    }
    if value.starts_with('/') {
        // Join to the base origin.
        let scheme_end = base.find("://")?;
        let after = &base[scheme_end + 3..];
        let host_end = after.find('/').unwrap_or(after.len());
        return Some(format!("{}://{}{}", &base[..scheme_end], &after[..host_end], value));
    }
    None
}

fn main() {
    // Keep the dhat profiler alive for the whole run; on drop it writes
    // dhat-heap.json with per-call-site allocation totals + peak.
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // ── Serve daemon mode ─────────────────────────────────────────────────────
    // The only async entry point. `main()` is otherwise synchronous, so we build
    // a multi-thread tokio runtime here and block on the daemon.
    if cli.serve {
        return run_serve(&cli);
    }

    // ── Whole-domain recon mode (crt.sh) ──────────────────────────────────────
    if let Some(domain) = cli.crtsh.clone() {
        return run_crtsh(&cli, &domain);
    }

    // ── Recursive recon-brain mode (the self-driving loop) ────────────────────
    if let Some(domain) = cli.recon.clone() {
        return run_recon(&cli, &domain);
    }

    // ── Asset-relationship graph mode ─────────────────────────────────────────
    if cli.relationships {
        let html = if cli.input == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            fs::read_to_string(&cli.input)?
        };
        let page_id = cli.base_url.clone().unwrap_or_else(|| cli.input.clone());
        let graph = js_beautify_rs::relationships::AssetGraph::from_html(&html, &page_id);
        let output_text = if cli.json {
            serde_json::to_string_pretty(&graph)?
        } else {
            let mut lines = vec![format!("Page: {}", graph.page)];
            for r in &graph.relations {
                let kind = serde_json::to_string(&r.kind).unwrap_or_default();
                let kind = kind.trim_matches('"');
                if matches!(r.kind, js_beautify_rs::relationships::RelationKind::InlineJs) {
                    lines.push(format!(
                        "[{kind}] (inline, {} bytes) line {}",
                        r.inline_content.len(),
                        r.line
                    ));
                } else {
                    lines.push(format!("[{kind}] {} line {}", r.target, r.line));
                }
            }
            lines.join("\n")
        };

        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
            eprintln!("Relationship graph written to {}", out_path.display());
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Sourcemap reversal mode ───────────────────────────────────────────────
    if let Some(ref map_path) = cli.reverse_sourcemap {
        let map_json =
            fs::read_to_string(map_path).map_err(|e| format!("failed to read {}: {}", map_path.display(), e))?;
        let reversed =
            js_beautify_rs::sourcemap::reverse(&map_json).map_err(|e| format!("failed to parse sourcemap: {e}"))?;

        if let Some(ref dir) = cli.reversed_sources_dir {
            fs::create_dir_all(dir)?;
            for rs in &reversed {
                // Sanitize source_path: strip leading `/` and `..` segments to
                // prevent escaping the output dir.
                let safe = rs.source_path.replace("..", "_").trim_start_matches('/').to_string();
                let dest = dir.join(&safe);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).ok();
                }
                fs::write(&dest, &rs.content)?;
            }
            eprintln!("Wrote {} reversed source(s) to {}", reversed.len(), dir.display());
        }

        // Run analyzers on each reversed source under FileType::ReversedSource.
        let cfg = AnalyzerConfig::default();
        let mut all_matches: Vec<js_beautify_rs::analyzers::AnalyzerMatch> = Vec::new();
        for rs in &reversed {
            let matches = run_all(
                &rs.content,
                Some(rs.source_path.as_str()),
                FileType::ReversedSource,
                &cfg,
            );
            all_matches.extend(matches);
        }

        if cli.unique {
            all_matches = js_beautify_rs::analyzers::dedup_unique_values(all_matches);
        }
        if let Some(ref min_sev_str) = cli.min_severity {
            if let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::from_str(min_sev_str) {
                all_matches.retain(|m| m.severity >= min_sev);
            }
        }

        let output_text = if cli.json {
            serde_json::to_string_pretty(&all_matches)?
        } else {
            let lines: Vec<String> = all_matches
                .iter()
                .map(|m| {
                    format!(
                        "[{}] {} @ {} (line {}:{})",
                        m.match_kind,
                        m.match_value,
                        m.file_path.as_deref().unwrap_or("?"),
                        m.position.start.line,
                        m.position.start.column
                    )
                })
                .collect();
            format!(
                "Reversed {} source(s); {} findings\n{}",
                reversed.len(),
                all_matches.len(),
                lines.join("\n")
            )
        };

        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
            eprintln!("Results written to {}", out_path.display());
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Directory scanning mode ───────────────────────────────────────────────
    if cli.dir {
        use std::path::Path;
        let dir = Path::new(&cli.input);
        if !dir.is_dir() {
            eprintln!("Error: '{}' is not a directory", cli.input);
            std::process::exit(1);
        }

        let extensions = ["js", "mjs", "cjs", "jsx", "ts", "tsx", "html", "htm"];
        let mut all_matches: Vec<js_beautify_rs::analyzers::AnalyzerMatch> = Vec::new();
        let mut file_count = 0u32;

        fn walk_dir(
            dir: &Path,
            extensions: &[&str],
            exclude: &[String],
            matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>,
            count: &mut u32,
        ) {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

                // Skip hidden dirs and node_modules
                if name.starts_with('.')
                    || name == "node_modules"
                    || name == "vendor"
                    || name == "dist"
                    || name == "build"
                {
                    continue;
                }

                // Check exclude patterns
                let path_str = path.to_string_lossy();
                if exclude.iter().any(|pat| path_str.contains(pat.as_str())) {
                    continue;
                }

                if path.is_dir() {
                    walk_dir(&path, extensions, exclude, matches, count);
                } else if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy().to_ascii_lowercase();
                    if extensions.iter().any(|e| *e == ext_str.as_str()) {
                        if let Ok(code) = std::fs::read_to_string(&path) {
                            let ft = if ext_str == "html" || ext_str == "htm" {
                                FileType::Html
                            } else {
                                FileType::Js
                            };
                            let cfg = AnalyzerConfig::default();
                            let file_matches = if ft == FileType::Html {
                                js_beautify_rs::analyzers::html_script_extractor::analyze_html(
                                    &code,
                                    Some(&path_str),
                                    &cfg,
                                )
                            } else {
                                run_all(&code, Some(&path_str), ft, &cfg)
                            };
                            matches.extend(file_matches);
                            *count += 1;
                        }
                    }
                }
            }
        }

        walk_dir(dir, &extensions, &cli.exclude, &mut all_matches, &mut file_count);

        // Apply scope filter
        if cli.in_scope.is_some() || cli.out_of_scope.is_some() || cli.scope_file.is_some() {
            let mut in_scope_combined = cli.in_scope.clone().unwrap_or_default();
            if let Some(ref sp) = cli.scope_file {
                if let Ok(c) = fs::read_to_string(sp) {
                    let fp: String = c
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .collect::<Vec<_>>()
                        .join(",");
                    if !in_scope_combined.is_empty() {
                        in_scope_combined.push(',');
                    }
                    in_scope_combined.push_str(&fp);
                }
            }
            let scope = js_beautify_rs::scope::ScopeMatcher::from_csv(
                cli.in_scope.as_deref().unwrap_or(""),
                cli.out_of_scope.as_deref().unwrap_or(""),
            );
            all_matches.retain(|m| {
                if m.match_kind == "hostname" || m.match_kind == "url" || m.match_kind == "comment_hostname" {
                    scope.is_in_scope(&m.match_value)
                } else {
                    true
                }
            });
        }

        // Apply severity filter
        if let Some(ref min_sev_str) = cli.min_severity {
            if let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::from_str(min_sev_str) {
                all_matches.retain(|m| m.severity >= min_sev);
            }
        }

        // Deduplicate
        if cli.unique {
            all_matches = js_beautify_rs::analyzers::dedup_unique_values(all_matches);
        }

        let output_text = if cli.json {
            serde_json::to_string_pretty(&all_matches)?
        } else {
            let mut lines: Vec<String> = all_matches
                .iter()
                .map(|m| {
                    format!(
                        "[{}] {} @ {} (line {}:{})",
                        m.match_kind,
                        m.match_value,
                        m.file_path.as_deref().unwrap_or("?"),
                        m.position.start.line,
                        m.position.start.column
                    )
                })
                .collect();
            lines.insert(
                0,
                format!("Scanned {} files, {} findings\n", file_count, all_matches.len()),
            );
            lines.join("\n")
        };

        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
            eprintln!(
                "Results written to {} ({} findings from {} files)",
                out_path.display(),
                all_matches.len(),
                file_count
            );
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }
    // ─────────────────────────────────────────────────────────────────────────

    let mut code = if cli.input == "-" {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        fs::read_to_string(&cli.input)?
    };

    let mut options = Options {
        // --rename-vars / --rename-llm imply deobfuscation (the renamer lives in
        // that pipeline; the LLM tier runs on its output).
        deobfuscate: cli.deobfuscate || cli.rename_vars || cli.rename_llm,
        readable_names: cli.rename_vars || cli.rename_llm,
        split_chunks: cli.split_chunks,
        extract_modules: cli.extract_modules,
        generate_source_map: cli.source_maps,
        indent_with_tabs: cli.indent_with_tabs,
        ..Options::default()
    };

    if let Some(dir) = cli.chunk_dir {
        options.chunk_dir = dir;
    }
    if let Some(path) = cli.chunk_map {
        options.chunk_map_output = Some(path);
    }
    if let Some(dir) = cli.module_dir {
        options.module_dir = dir;
    }
    if let Some(path) = &cli.dependency_graph {
        options.dependency_graph = Some(path.clone());
    }
    if let Some(size) = cli.indent_size {
        options.indent_size = size;
    }

    if options.extract_modules {
        eprintln!("[WEBPACK] Extracting modules...");
        let mut tokenizer = Tokenizer::new(&code);
        let tokens = tokenizer.tokenize()?;
        let mut extractor = ModuleExtractor::new();
        extractor.extract_modules(&tokens)?;
        extractor.extract_dependencies(&tokens)?;

        eprintln!("[WEBPACK] Found {} modules", extractor.module_count());

        extractor.write_modules(&tokens, &options.module_dir)?;
        eprintln!("[WEBPACK] Modules written to {}", options.module_dir.display());

        if let Some(graph_path) = &options.dependency_graph {
            extractor.generate_dependency_graph(graph_path)?;
            eprintln!("[WEBPACK] Dependency graph written to {}", graph_path.display());
        }

        return Ok(());
    }

    // `--rename-vars --sourcemap x.map` (without an align target): recover real
    // original names from the sourcemap first, then fall through to the normal
    // readable deobfuscate pipeline (which names whatever the map didn't cover).
    let sourcemap_rename = (cli.rename_vars || cli.rename_llm) && cli.sourcemap.is_some() && cli.align_with.is_none();
    if sourcemap_rename {
        let sm = fs::read_to_string(cli.sourcemap.as_ref().expect("sourcemap present"))?;
        eprintln!("[RENAME] Applying sourcemap original names...");
        code = js_beautify_rs::ast_deobfuscate::sourcemap_rename::apply_sourcemap_names(&code, &sm)?;
    }

    if !sourcemap_rename
        && (cli.align_with.is_some() || cli.sourcemap.is_some() || cli.names_json.is_some() || cli.bun_extract)
    {
        options.skip_annotations = true;
        return run_cross_version_align(
            cli.sourcemap.as_ref(),
            cli.names_json.as_ref(),
            cli.align_with.as_ref(),
            cli.align_output.as_ref(),
            cli.output.as_ref(),
            &code,
            &options,
            cli.raw,
            cli.bun_extract,
        );
    }

    // ── Security analysis / wordlist ──────────────────────────────────────────
    let run_analysis = cli.analyze
        || cli.analyze_all
        || cli.scan_secrets
        || cli.find_sinks
        || cli.extract_paths
        || cli.extract_hostnames
        || cli.extract_graphql
        || cli.extract_packages
        || cli.find_sensitive_props;

    if run_analysis {
        let cfg = if cli.analyze || cli.analyze_all {
            AnalyzerConfig::default()
        } else {
            AnalyzerConfig {
                secrets: cli.scan_secrets,
                sensitive_props: cli.find_sensitive_props,
                paths_hostnames: cli.extract_paths || cli.extract_hostnames,
                graphql: cli.extract_graphql,
                sinks: cli.find_sinks,
                npm_packages: cli.extract_packages,
                comments: cli.extract_paths || cli.extract_hostnames,
                framework_recon: false,
                // Preserve prior behaviour: API/storage extraction always ran in
                // the CLI path regardless of the targeted flags.
                api_calls: true,
                // Entropy + taint + idor follow the secret/sink intent of this scan.
                entropy_secrets: cli.scan_secrets,
                taint: cli.find_sinks,
                idor: cli.extract_paths || cli.find_sinks,
                proto_pollution: cli.find_sinks,
                resolve_endpoints: cli.extract_paths,
            }
        };

        let file_path = if cli.input == "-" {
            None
        } else {
            Some(cli.input.as_str())
        };
        let mut matches = run_all(&code, file_path, FileType::Js, &cfg);

        // --vuln-scan: map detected npm packages → known CVEs via OSV + KEV.
        if cli.vuln_scan {
            matches.extend(run_vuln_scan(&matches, file_path));
        }

        // --extract-hostnames (without --extract-paths) keeps only host kinds
        if cli.extract_hostnames && !cli.extract_paths && !cli.analyze && !cli.analyze_all {
            matches.retain(|m| matches!(m.match_kind.as_str(), "hostname" | "comment_hostname"));
        }

        // Apply scope filtering if --in-scope is set
        if cli.in_scope.is_some() || cli.out_of_scope.is_some() || cli.scope_file.is_some() {
            let mut in_scope_combined = cli.in_scope.clone().unwrap_or_default();
            if let Some(ref sp) = cli.scope_file {
                if let Ok(c) = fs::read_to_string(sp) {
                    let fp: String = c
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .collect::<Vec<_>>()
                        .join(",");
                    if !in_scope_combined.is_empty() {
                        in_scope_combined.push(',');
                    }
                    in_scope_combined.push_str(&fp);
                }
            }
            let scope = js_beautify_rs::scope::ScopeMatcher::from_csv(
                cli.in_scope.as_deref().unwrap_or(""),
                cli.out_of_scope.as_deref().unwrap_or(""),
            );
            matches.retain(|m| {
                // Only filter hostname/url matches by scope
                if m.match_kind == "hostname" || m.match_kind == "url" || m.match_kind == "comment_hostname" {
                    scope.is_in_scope(&m.match_value)
                } else {
                    true
                }
            });
        }

        // Apply derived analyzers if --derive is set
        if !cli.derive.is_empty() {
            for spec in &cli.derive {
                // Format: "new_kind:original_kind:include_regex"
                let parts: Vec<&str> = spec.splitn(3, ':').collect();
                if parts.len() >= 2 {
                    let new_kind = parts[0];
                    let original_kind = parts[1];
                    let include = if parts.len() == 3 { vec![parts[2]] } else { vec![] };
                    if let Ok(derived) =
                        js_beautify_rs::analyzers::derived::DerivedAnalyzer::new(original_kind, new_kind, &include, &[])
                    {
                        let derived_matches = derived.apply(&matches);
                        matches.extend(derived_matches);
                    }
                }
            }
        }

        // Apply severity filter
        if let Some(ref min_sev_str) = cli.min_severity {
            if let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::from_str(min_sev_str) {
                matches.retain(|m| m.severity >= min_sev);
            }
        }

        // Deduplicate by unique value if --unique
        if cli.unique {
            matches = js_beautify_rs::analyzers::dedup_unique_values(matches);
        }

        // Wordlist from matches mode
        if cli.wordlist_from_matches {
            let words: std::collections::HashSet<String> = matches
                .iter()
                .filter(|m| {
                    matches!(
                        m.match_kind.as_str(),
                        "path" | "url" | "hostname" | "api_call" | "comment_path" | "comment_hostname"
                    )
                })
                .flat_map(|m| {
                    m.match_value
                        .split(|c| matches!(c, '/' | '?' | '=' | '&' | '#' | ':' | '-' | '_' | '.'))
                        .filter(|s| s.len() > 1)
                        .map(|s| s.to_ascii_lowercase())
                })
                .filter(|w| !js_beautify_rs::wordlist::filter::is_noise(w))
                .collect();
            let mut words: Vec<_> = words.into_iter().collect();
            words.sort_unstable();
            let output_text = words.join("\n");
            if let Some(out_path) = &cli.analysis_out {
                fs::write(out_path, &output_text)?;
                eprintln!("Wordlist from matches: {} words → {}", words.len(), out_path.display());
            } else {
                println!("{output_text}");
            }
            return Ok(());
        }

        // SARIF output
        if cli.sarif {
            let sarif = serde_json::json!({
                "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
                "version": "2.1.0",
                "runs": [{
                    "tool": {
                        "driver": {
                            "name": "jsbeautify",
                            "version": env!("CARGO_PKG_VERSION"),
                            "informationUri": "https://github.com/user/js-beautify-rs",
                            "rules": matches.iter().map(|m| &m.match_kind).collect::<std::collections::HashSet<_>>().iter().map(|kind| {
                                serde_json::json!({
                                    "id": kind,
                                    "shortDescription": { "text": kind },
                                    "defaultConfiguration": {
                                        "level": match js_beautify_rs::analyzers::regex_bank::severity_for_kind(kind) {
                                            js_beautify_rs::analyzers::regex_bank::Severity::Critical | js_beautify_rs::analyzers::regex_bank::Severity::High => "error",
                                            js_beautify_rs::analyzers::regex_bank::Severity::Medium => "warning",
                                            _ => "note",
                                        }
                                    }
                                })
                            }).collect::<Vec<_>>()
                        }
                    },
                    "results": matches.iter().map(|m| {
                        serde_json::json!({
                            "ruleId": m.match_kind,
                            "level": match js_beautify_rs::analyzers::regex_bank::severity_for_kind(&m.match_kind) {
                                js_beautify_rs::analyzers::regex_bank::Severity::Critical | js_beautify_rs::analyzers::regex_bank::Severity::High => "error",
                                js_beautify_rs::analyzers::regex_bank::Severity::Medium => "warning",
                                _ => "note",
                            },
                            "message": { "text": m.match_value },
                            "locations": [{
                                "physicalLocation": {
                                    "artifactLocation": { "uri": m.file_path.as_deref().unwrap_or("stdin") },
                                    "region": {
                                        "startLine": m.position.start.line,
                                        "startColumn": m.position.start.column + 1,
                                        "endLine": m.position.end.line,
                                        "endColumn": m.position.end.column + 1,
                                    }
                                }
                            }]
                        })
                    }).collect::<Vec<_>>()
                }]
            });
            let output_text = serde_json::to_string_pretty(&sarif)?;
            if let Some(out_path) = &cli.analysis_out {
                fs::write(out_path, &output_text)?;
            } else {
                println!("{output_text}");
            }
            return Ok(());
        }

        // --chains / --agent: correlate findings into attack chains, optionally
        // letting an LLM agent investigate + write a narrative report.
        let attack_chains = if cli.chains || cli.agent {
            js_beautify_rs::chains::correlate(&matches)
        } else {
            Vec::new()
        };
        let agent_report = if cli.agent { run_agent(&matches, &code) } else { None };

        // --validate: actively confirm findings against the live (scoped) target.
        let validations = if cli.validate {
            run_validation(cli.validate_scope.as_deref(), &matches)
        } else {
            Vec::new()
        };

        let output_text = if cli.json {
            serde_json::to_string_pretty(&serde_json::json!({
                "findings": matches,
                "chains": attack_chains,
                "agent": agent_report,
                "validations": validations,
            }))?
        } else {
            let mut lines = Vec::with_capacity(matches.len());
            for m in &matches {
                lines.push(format!(
                    "[{}] {} (line {}:{})",
                    m.match_kind, m.match_value, m.position.start.line, m.position.start.column
                ));
            }
            if lines.is_empty() {
                lines.push("No findings.".to_string());
            }
            if !attack_chains.is_empty() {
                lines.push(format!("\n=== Attack chains ({}) ===", attack_chains.len()));
                for c in &attack_chains {
                    lines.push(format!("  [{}/score {}] {} — {}", c.kind, c.score, c.cwe, c.title));
                }
            }
            if let Some(rep) = &agent_report {
                lines.push(format!("\n=== Agent report ({} turns) ===\n{}", rep.turns, rep.report));
            }
            if !validations.is_empty() {
                lines.push(format!("\n=== Live validation ({}) ===", validations.len()));
                for v in &validations {
                    lines.push(format!("  [{}] {} → {}", v.verdict_str, v.target, v.evidence));
                }
            }
            lines.join("\n")
        };

        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
            eprintln!("Analysis written to {}", out_path.display());
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    if cli.wordlist {
        use js_beautify_rs::wordlist::extractors::html_extractor::{self as html_ext, HtmlExtractConfig};
        use js_beautify_rs::wordlist::extractors::js_ast_extractor::{self, ExtractConfig};

        let min_len = cli.min_length.unwrap_or(2);
        let max_len = cli.max_length.unwrap_or(0);
        let limit = cli.limit.unwrap_or(0);

        // Collect sourced words
        let mut all_words: Vec<(String, &str)> = Vec::new();

        // Detect if input is HTML
        let is_html = code.trim_start().starts_with('<') || cli.input.ends_with(".html") || cli.input.ends_with(".htm");

        if is_html {
            for w in html_ext::extract_sourced(&code, &HtmlExtractConfig::default()) {
                all_words.push((w.word, w.source));
            }
            // Also extract from inline scripts
            for block in js_beautify_rs::analyzers::html_script_extractor::extract_scripts(&code) {
                for w in js_ast_extractor::extract_sourced(&block.content, &ExtractConfig::default()) {
                    all_words.push((w.word, w.source));
                }
            }
        } else {
            for w in js_ast_extractor::extract_sourced(&code, &ExtractConfig::default()) {
                all_words.push((w.word, w.source));
            }
        }

        // Filter by sources
        if !cli.sources.is_empty() {
            all_words.retain(|(_, src)| cli.sources.iter().any(|s| s == *src));
        }

        // Apply length filters
        all_words.retain(|(w, _)| w.len() >= min_len && (max_len == 0 || w.len() <= max_len));

        // Lowercase
        if cli.lowercase {
            for (w, _) in &mut all_words {
                *w = w.to_ascii_lowercase();
            }
        }

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        all_words.retain(|(w, _)| seen.insert(w.clone()));

        // Plurals
        if cli.plurals {
            let mut extras: Vec<(String, &str)> = Vec::new();
            for (w, src) in &all_words {
                if w.ends_with('s') {
                    // Generate singular
                    if w.len() > 3 {
                        extras.push((w[..w.len() - 1].to_string(), *src));
                    }
                } else {
                    // Generate plural
                    extras.push((format!("{w}s"), *src));
                }
            }
            for e in extras {
                if seen.insert(e.0.clone()) {
                    all_words.push(e);
                }
            }
        }

        // Sort
        if cli.sort_by_count {
            // We don't have occurrence count without a DB, so just sort alpha
            all_words.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        } else {
            all_words.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }

        // Limit
        if limit > 0 && all_words.len() > limit {
            all_words.truncate(limit);
        }

        let output_text = if cli.json {
            let items: Vec<serde_json::Value> = all_words
                .iter()
                .map(|(w, src)| serde_json::json!({"word": w, "source": src}))
                .collect();
            serde_json::to_string_pretty(&items)?
        } else {
            all_words.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>().join("\n")
        };

        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
            eprintln!("Wordlist written to {} ({} words)", out_path.display(), all_words.len());
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Custom regex scan ────────────────────────────────────────────────────
    if !cli.custom_regex.is_empty() {
        let mut patterns: Vec<(String, regex::Regex)> = Vec::new();
        for spec in &cli.custom_regex {
            let (label, pat) = spec.split_once(':').unwrap_or(("custom", spec));
            match regex::Regex::new(pat) {
                Ok(re) => patterns.push((label.to_string(), re)),
                Err(e) => {
                    eprintln!("Invalid regex '{}': {}", pat, e);
                    std::process::exit(1);
                }
            }
        }
        let file_path = if cli.input == "-" {
            None
        } else {
            Some(cli.input.as_str())
        };
        let matches = js_beautify_rs::analyzers::secret_scanner::scan_custom(&code, file_path, FileType::Js, &patterns);
        let output_text = if cli.json {
            serde_json::to_string_pretty(&matches)?
        } else {
            matches
                .iter()
                .map(|m| {
                    format!(
                        "[{}] {} (line {}:{})",
                        m.match_kind, m.match_value, m.position.start.line, m.position.start.column
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Analyze HTML (inline scripts) ────────────────────────────────────────
    if cli.analyze_html {
        let matches = js_beautify_rs::analyzers::html_script_extractor::analyze_html(
            &code,
            if cli.input == "-" {
                None
            } else {
                Some(cli.input.as_str())
            },
            &AnalyzerConfig::default(),
        );
        let output_text = if cli.json {
            serde_json::to_string_pretty(&matches)?
        } else {
            if matches.is_empty() {
                "No findings in inline scripts.".to_string()
            } else {
                matches
                    .iter()
                    .map(|m| {
                        format!(
                            "[{}] {} (line {}:{})",
                            m.match_kind, m.match_value, m.position.start.line, m.position.start.column
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };
        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Next.js chunk discovery ──────────────────────────────────────────────
    if cli.discover_nextjs {
        let base = cli.base_url.as_deref().unwrap_or("https://example.com");
        let result = js_beautify_rs::nextjs_discovery::discover_from_html(&code, base);
        let output_text = if cli.json {
            serde_json::to_string_pretty(&serde_json::json!({
                "build_id": result.build_id,
                "base_url": result.base_url,
                "chunk_urls": result.chunk_urls,
                "routes": result.routes,
            }))?
        } else {
            let mut lines = Vec::new();
            if !result.build_id.is_empty() {
                lines.push(format!("Build ID: {}", result.build_id));
            }
            lines.push(format!("Chunks discovered: {}", result.chunk_urls.len()));
            for url in &result.chunk_urls {
                lines.push(format!("  {url}"));
            }
            if !result.routes.is_empty() {
                lines.push(format!("\nRoutes ({}):", result.routes.len()));
                for r in &result.routes {
                    lines.push(format!("  {r}"));
                }
            }
            lines.join("\n")
        };
        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }
    // ─────────────────────────────────────────────────────────────────────────

    // ── Repeater ─────────────────────────────────────────────────────────────
    if cli.repeat {
        let config = js_beautify_rs::repeater::RepeaterConfig {
            url: cli.repeat_url.clone(),
            proxy_host: cli.proxy_host.clone(),
            proxy_port: cli.proxy_port,
            update_content_length: true,
            timeout_secs: 30,
        };
        let result = js_beautify_rs::repeater::replay(&code, &config)?;
        let output_text = if cli.json {
            serde_json::to_string_pretty(&result)?
        } else {
            format!(
                "HTTP {} | {} bytes | {}ms\n{}",
                result.status_code, result.body_size, result.elapsed_ms, result.body
            )
        };
        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Fuzz ─────────────────────────────────────────────────────────────────
    if cli.fuzz {
        use js_beautify_rs::fuzz::*;

        let mode = cli
            .attack_mode
            .as_deref()
            .and_then(AttackMode::from_str)
            .unwrap_or(AttackMode::Sniper);

        // Determine markers
        let markers: Vec<String> = if cli.marker.is_empty() {
            vec!["FUZZ".into(), "FUZZ2".into(), "FUZZ3".into()]
        } else {
            cli.marker.clone()
        };

        // Load wordlists
        let mut wordlists: Vec<Vec<String>> = Vec::new();
        for wl_path in &cli.fuzz_wordlist {
            wordlists.push(load_wordlist(wl_path)?);
        }
        if let Some(ref payloads_str) = cli.payloads {
            let inline: Vec<String> = payloads_str.split(',').map(|s| s.trim().to_string()).collect();
            if wordlists.is_empty() {
                wordlists.push(inline);
            } else {
                wordlists[0].extend(inline);
            }
        }
        if wordlists.is_empty() {
            eprintln!("Error: --fuzz requires --fuzz-wordlist or --payloads");
            std::process::exit(1);
        }

        let combinations = generate_combinations(&wordlists, mode);
        let concurrency = cli.concurrency.unwrap_or(10);
        let delay = cli.delay_ms.unwrap_or(0);
        let match_statuses: Vec<u16> = cli
            .match_status
            .as_deref()
            .map(|s| s.split(',').filter_map(|c| c.trim().parse().ok()).collect())
            .unwrap_or_default();

        let config = js_beautify_rs::repeater::RepeaterConfig {
            url: cli.repeat_url.clone(),
            proxy_host: cli.proxy_host.clone(),
            proxy_port: cli.proxy_port,
            update_content_length: true,
            timeout_secs: 30,
        };

        eprintln!(
            "[FUZZ] Mode: {mode}, Combinations: {}, Concurrency: {concurrency}",
            combinations.len()
        );

        let mut results: Vec<js_beautify_rs::fuzz::FuzzResult> = Vec::new();
        for (idx, combo) in combinations.iter().enumerate() {
            let rendered = apply_payloads(&code, &markers, combo);
            match js_beautify_rs::repeater::replay(&rendered, &config) {
                Ok(resp) => {
                    // Filter by status
                    if !match_statuses.is_empty() && !match_statuses.contains(&resp.status_code) {
                        continue;
                    }
                    // Filter by body content
                    if let Some(ref filter) = cli.filter_body {
                        if resp.body.contains(filter.as_str()) {
                            continue;
                        }
                    }
                    let preview = resp.body.chars().take(80).collect::<String>();
                    if !cli.json {
                        println!(
                            "[{}] {:?} → {} | {} bytes | {}ms | {}",
                            idx,
                            combo,
                            resp.status_code,
                            resp.body_size,
                            resp.elapsed_ms,
                            preview.replace('\n', " ")
                        );
                    }
                    results.push(FuzzResult {
                        index: idx,
                        payloads: combo.clone(),
                        status_code: resp.status_code,
                        response_size: resp.body_size,
                        body_preview: preview,
                    });
                }
                Err(e) => {
                    if !cli.json {
                        eprintln!("[{}] {:?} → ERROR: {}", idx, combo, e);
                    }
                }
            }
            if delay > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results)?);
        } else {
            eprintln!(
                "\n[FUZZ] Done. {} results ({} filtered out)",
                results.len(),
                combinations.len() - results.len()
            );
        }
        return Ok(());
    }

    // ── Diff ─────────────────────────────────────────────────────────────────
    if let Some(ref old_path) = cli.diff_with {
        let old_code = fs::read_to_string(old_path)?;
        let result = js_beautify_rs::diff::diff_versions(
            &old_code,
            &code,
            if cli.input == "-" {
                None
            } else {
                Some(cli.input.as_str())
            },
            FileType::Js,
        );
        let output_text = if cli.json {
            serde_json::to_string_pretty(&result)?
        } else {
            let mut lines = Vec::new();
            lines.push(format!("Content changed: {}", result.content_changed));
            lines.push(format!("Old hash: {}", result.old_hash));
            lines.push(format!("New hash: {}", result.new_hash));
            if !result.added.is_empty() {
                lines.push(format!("\n+++ ADDED ({}):", result.added.len()));
                for m in &result.added {
                    lines.push(format!("  + [{}] {}", m.match_kind, m.match_value));
                }
            }
            if !result.removed.is_empty() {
                lines.push(format!("\n--- REMOVED ({}):", result.removed.len()));
                for m in &result.removed {
                    lines.push(format!("  - [{}] {}", m.match_kind, m.match_value));
                }
            }
            lines.push(format!("\nUnchanged findings: {}", result.unchanged_count));
            lines.join("\n")
        };
        if let Some(out_path) = &cli.analysis_out {
            fs::write(out_path, &output_text)?;
        } else {
            println!("{output_text}");
        }
        return Ok(());
    }

    // ── Scope filtering on analyze results ───────────────────────────────────
    // (handled inline when --in-scope is combined with --analyze)

    let mut beautified = beautify(&code, &options)?;

    // Never drop input: if beautification/deobfuscation yields nothing for a
    // non-empty source (e.g. an HTTP-response dump or otherwise unparseable
    // text), fall back to emitting the original so output is never silently
    // empty.
    if beautified.trim().is_empty() && !code.trim().is_empty() {
        eprintln!("[jsbeautify] empty output after processing; emitting original source unchanged");
        beautified = code.clone();
    }

    // Final LLM tier: name residual minified locals the heuristics couldn't.
    if cli.rename_llm {
        let overrides = js_beautify_rs::llm::LlmOverrides {
            url: cli.llm_url.clone(),
            model: cli.llm_model.clone(),
            key: cli.llm_key.clone(),
        };
        match js_beautify_rs::ast_deobfuscate::llm_rename::OpenAiCompatSuggester::from_overrides(&overrides) {
            Some(suggester) => {
                eprintln!("[RENAME] Querying LLM for residual minified names...");
                beautified = js_beautify_rs::ast_deobfuscate::llm_rename::apply_llm_names(&beautified, &suggester);
            }
            None => eprintln!(
                "[RENAME] --rename-llm set but no endpoint configured \
                 (set OPENAI_API_KEY, or --llm-url / JSBEAUTIFY_LLM_URL); skipping LLM tier."
            ),
        }
    }

    if let Some(output_path) = &cli.output {
        fs::write(output_path, &beautified)?;
        eprintln!("Beautified code written to {output_path}");
    } else {
        println!("{beautified}");
    }

    Ok(())
}

fn run_cross_version_align(
    sourcemap_path: Option<&PathBuf>,
    names_json_path: Option<&PathBuf>,
    align_with_path: Option<&PathBuf>,
    align_output_path: Option<&PathBuf>,
    output_path: Option<&String>,
    source_code: &str,
    options: &Options,
    raw: bool,
    bun_extract: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AlignConfig {
        sourcemap_path: sourcemap_path.map(|p| p.to_string_lossy().to_string()),
        align_with: align_with_path.map(|p| p.to_string_lossy().to_string()),
        hash_depth: 12,
        use_slot_naming: false,
    };

    let mut aligner = CrossVersionAligner::new(config);

    if let Some(names_path) = names_json_path {
        eprintln!("[ALIGN] Loading names.json: {}", names_path.display());
        let names_json = fs::read_to_string(names_path)?;
        let stable_count = aligner.load_names_json(&names_json)?;
        eprintln!("[ALIGN] Loaded {} stable name mappings from names.json", stable_count);
    } else if let Some(smap_path) = sourcemap_path {
        eprintln!("[ALIGN] Loading sourcemap: {}", smap_path.display());
        let sourcemap_json = fs::read_to_string(smap_path)?;
        let stable_count = aligner.load_sourcemap(&sourcemap_json, source_code)?;
        eprintln!("[ALIGN] Loaded {} stable name mappings from sourcemap", stable_count);
    }

    if bun_extract {
        eprintln!("[ALIGN] Extracting names from Bun bundle patterns...");
        let bun_count = aligner.load_bun_names(source_code);
        eprintln!(
            "[ALIGN] Extracted {} name mappings from MR/this.name/displayName",
            bun_count
        );
    }

    if let Some(target_path) = align_with_path {
        eprintln!("[ALIGN] Loading target bundle: {}", target_path.display());
        let target_code = fs::read_to_string(target_path)?;

        eprintln!("[ALIGN] Aligning statements on RAW code...");
        let (aligned_source, aligned_target, stats) = aligner.align_sources(source_code, &target_code);

        eprintln!("[ALIGN] === Results ===");
        eprintln!(
            "[ALIGN] Matched: {} / {} ({:.1}%)",
            stats.matched_statements,
            stats.source_statements,
            stats.match_rate()
        );
        eprintln!("[ALIGN] Source replacements: {}", stats.source_replacements);
        eprintln!("[ALIGN] Target replacements: {}", stats.target_replacements);
        eprintln!("[ALIGN] Canonical names generated: {}", stats.canonical_names_generated);

        let (source_out, target_out) = if raw {
            (aligned_source, aligned_target)
        } else {
            eprintln!("[ALIGN] Beautifying aligned source...");
            let source_beautified = beautify(&aligned_source, options)?;
            eprintln!("[ALIGN] Source beautified: {} lines", source_beautified.lines().count());

            eprintln!("[ALIGN] Beautifying aligned target...");
            let target_beautified = beautify(&aligned_target, options)?;
            eprintln!("[ALIGN] Target beautified: {} lines", target_beautified.lines().count());
            (source_beautified, target_beautified)
        };

        if let Some(align_out) = align_output_path {
            fs::write(align_out, &target_out)?;
            eprintln!("[ALIGN] Target output written to {}", align_out.display());
        }

        if let Some(out_path) = output_path {
            fs::write(out_path, &source_out)?;
            eprintln!("[ALIGN] Source output written to {out_path}");
        } else {
            println!("{source_out}");
        }
    } else {
        eprintln!("[ALIGN] Beautifying source...");
        let source_beautified = beautify(source_code, options)?;
        eprintln!("[ALIGN] Source beautified: {} lines", source_beautified.lines().count());

        if let Some(out_path) = output_path {
            fs::write(out_path, &source_beautified)?;
            eprintln!("[ALIGN] Source output written to {out_path}");
        } else {
            println!("{source_beautified}");
        }
    }

    Ok(())
}
