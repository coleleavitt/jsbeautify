use super::Cli;
use js_beautify_rs::analyzers::AnalyzerConfig;
use js_beautify_rs::analyzers::match_kind::FileType;
use std::fs;
use std::path::PathBuf;

pub(super) fn run_custom_regex(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut patterns: Vec<(String, regex::Regex)> = Vec::new();
    for spec in &cli.custom_regex {
        let (label, pat) = spec.split_once(':').unwrap_or(("custom", spec));
        match regex::Regex::new(pat) {
            Ok(re) => patterns.push((label.to_string(), re)),
            Err(e) => {
                eprintln!("Invalid regex '{pat}': {e}");
                std::process::exit(1);
            }
        }
    }
    let file_path = (cli.input != "-").then_some(cli.input.as_str());
    let matches = js_beautify_rs::analyzers::secret_scanner::scan_custom(code, file_path, FileType::Js, &patterns);
    output_matches(cli, &matches)
}

pub(super) fn run_analyze_html(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let matches = js_beautify_rs::analyzers::html_script_extractor::analyze_html(
        code,
        (cli.input != "-").then_some(cli.input.as_str()),
        &AnalyzerConfig::default(),
    );
    if matches.is_empty() && !cli.json {
        output_text(cli, "No findings in inline scripts.")
    } else {
        output_matches(cli, &matches)
    }
}

pub(super) fn run_nextjs_discovery(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let base = cli.base_url.as_deref().unwrap_or("https://example.com");
    let result = js_beautify_rs::nextjs_discovery::discover_from_html(code, base);
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
            for route in &result.routes {
                lines.push(format!("  {route}"));
            }
        }
        lines.join("\n")
    };
    output_owned(cli, &output_text)
}

pub(super) fn run_diff(cli: &Cli, code: &str, old_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let old_code = fs::read_to_string(old_path)?;
    let result = js_beautify_rs::diff::diff_versions(
        &old_code,
        code,
        (cli.input != "-").then_some(cli.input.as_str()),
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
    output_owned(cli, &output_text)
}

fn output_matches(
    cli: &Cli,
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
) -> Result<(), Box<dyn std::error::Error>> {
    let output_text = if cli.json {
        serde_json::to_string_pretty(matches)?
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
    output_owned(cli, &output_text)
}

fn output_text(cli: &Cli, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    output_owned(cli, text)
}

fn output_owned(cli: &Cli, output_text: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(out_path) = &cli.analysis_out {
        fs::write(out_path, output_text)?;
    } else {
        println!("{output_text}");
    }
    Ok(())
}
