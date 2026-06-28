// Heap profiling: with `--features dhat-heap`, dhat replaces the global
// allocator and records every allocation's call site (see Cargo.toml).
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use clap::Parser;
use std::path::PathBuf;

mod align;
mod analysis;
mod directory;
mod recon;
mod runner;
mod scan;
mod sources;
mod wordlist;

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
    /// (humanify pattern). Works with a standard `OPENAI_API_KEY`, or any
    /// OpenAI-compatible endpoint via --llm-url / env. Implies --rename-vars.
    #[arg(long)]
    rename_llm: bool,

    /// OpenAI-compatible endpoint URL (overrides `OPENAI_BASE_URL` /
    /// `JSBEAUTIFY_LLM_URL`). A bare base gets `/chat/completions` appended.
    #[arg(long, value_name = "URL", global = true)]
    llm_url: Option<String>,

    /// LLM model id (overrides `OPENAI_MODEL` / `JSBEAUTIFY_LLM_MODEL`).
    #[arg(long, value_name = "MODEL", global = true)]
    llm_model: Option<String>,

    /// LLM API key (overrides `OPENAI_API_KEY` / `JSBEAUTIFY_LLM_KEY`).
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

    /// Reconstruct the original source tree from a .js.map into <dir>/reversed/
    /// (walks sourcesContent, normalizes webpack:// paths). Value is the output dir.
    #[arg(long, value_name = "DIR")]
    reconstruct_sourcemap: Option<String>,

    /// Reconstruct the original source tree from this JS bundle's sourceMappingURL
    /// into <dir>/reversed/. Supports inline and local external source maps.
    #[arg(long, value_name = "DIR")]
    split_sources: Option<String>,

    // ── Bundle analysis ──────────────────────────────────────────────────────
    /// Run all passive bundle analyzers (secrets, sinks, paths, GraphQL, npm packages)
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

    /// Detect sensitive property names (secret, authorization, `aws_secret_key`, …)
    #[arg(long)]
    find_sensitive_props: bool,

    /// Generate a source-derived wordlist from JS identifiers and string literals
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

    /// Filter wordlist by extraction source (e.g. `js_variable_name`, `html_body_text`). Repeatable.
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

    // ── Diff ─────────────────────────────────────────────────────────────────
    /// Diff two JS/HTML files and show added/removed analyzer findings
    #[arg(long, value_name = "OLD_FILE")]
    diff_with: Option<PathBuf>,

    // ── Scope ────────────────────────────────────────────────────────────────
    /// In-scope patterns (comma-separated hostnames/globs) for filtering results
    #[arg(long, value_name = "PATTERNS")]
    in_scope: Option<String>,

    /// Out-of-scope patterns (comma-separated) — exclusions
    #[arg(long, value_name = "PATTERNS")]
    out_of_scope: Option<String>,

    /// Derived analyzer: "`new_kind:original_kind:include_regex`" (repeatable)
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

    /// Deduplicate matches by (`match_kind`, `match_value`, `file_path`, metadata).
    /// Matches jxscout's `finding_per_unique_value` setting.
    #[arg(long)]
    unique: bool,

    /// Run all passive bundle analyzers (alias of --analyze).
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
    /// `js_loader` / `iframe_loader` / lazy-chunk graph). Use --base-url to
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

pub fn main() {
    runner::main();
}
