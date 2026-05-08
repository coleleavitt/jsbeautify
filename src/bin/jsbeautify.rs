use clap::Parser;
use js_beautify_rs::analyzers::match_kind::FileType;
use js_beautify_rs::analyzers::{AnalyzerConfig, run_all};
use js_beautify_rs::cross_version::{AlignConfig, CrossVersionAligner};
use js_beautify_rs::tokenizer::Tokenizer;
use js_beautify_rs::webpack_module_extractor::ModuleExtractor;
use js_beautify_rs::{Options, beautify, wordlist};
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
    /// Input JavaScript file (use "-" for stdin)
    #[arg(value_name = "FILE")]
    input: String,

    /// Write output to a file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Enable AST-based deobfuscation (19-phase pipeline)
    #[arg(short, long)]
    deobfuscate: bool,

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

    /// Extract GraphQL operations
    #[arg(long)]
    extract_graphql: bool,

    /// Extract npm package references
    #[arg(long)]
    extract_packages: bool,

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

    /// Base URL for Next.js chunk discovery (required with --discover-nextjs)
    #[arg(long, value_name = "URL")]
    base_url: Option<String>,

    /// Output analysis results as JSON (default: human-readable)
    #[arg(long)]
    json: bool,

    /// Write analysis/wordlist output to FILE instead of stdout
    #[arg(long, value_name = "FILE")]
    analysis_out: Option<PathBuf>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let code = if cli.input == "-" {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        fs::read_to_string(&cli.input)?
    };

    let mut options = Options {
        deobfuscate: cli.deobfuscate,
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

    if cli.align_with.is_some() || cli.sourcemap.is_some() || cli.names_json.is_some() || cli.bun_extract {
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
        || cli.scan_secrets
        || cli.find_sinks
        || cli.extract_paths
        || cli.extract_graphql
        || cli.extract_packages
        || cli.find_sensitive_props;

    if run_analysis {
        let cfg = if cli.analyze {
            AnalyzerConfig::default()
        } else {
            AnalyzerConfig {
                secrets: cli.scan_secrets,
                sensitive_props: cli.find_sensitive_props,
                paths_hostnames: cli.extract_paths,
                graphql: cli.extract_graphql,
                sinks: cli.find_sinks,
                npm_packages: cli.extract_packages,
                comments: cli.extract_paths, // include comments when paths requested
            }
        };

        let file_path = if cli.input == "-" {
            None
        } else {
            Some(cli.input.as_str())
        };
        let matches = run_all(&code, file_path, FileType::Js, &cfg);

        let output_text = if cli.json {
            serde_json::to_string_pretty(&matches)?
        } else {
            let mut lines = Vec::with_capacity(matches.len());
            for m in &matches {
                lines.push(format!(
                    "[{}] {} (line {}:{})",
                    m.match_kind, m.match_value, m.position.start.line, m.position.start.column
                ));
            }
            if lines.is_empty() {
                "No findings.".to_string()
            } else {
                lines.join("\n")
            }
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
        use js_beautify_rs::wordlist::extractors::html_extractor::{
            self as html_ext, HtmlExtractConfig, HtmlSourcedWord,
        };
        use js_beautify_rs::wordlist::extractors::js_ast_extractor::{self, ExtractConfig, SourcedWord};

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

    let beautified = beautify(&code, &options)?;

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
