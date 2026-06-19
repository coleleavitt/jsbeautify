use super::Cli;
use js_beautify_rs::analyzers::match_kind::FileType;
use js_beautify_rs::analyzers::{AnalyzerConfig, run_all};
use std::fs;
use std::path::Path;

pub(super) fn run_directory_scan(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(&cli.input);
    if !dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", cli.input);
        std::process::exit(1);
    }

    let extensions = ["js", "mjs", "cjs", "jsx", "ts", "tsx", "html", "htm"];
    let mut all_matches: Vec<js_beautify_rs::analyzers::AnalyzerMatch> = Vec::new();
    let mut file_count = 0u32;

    walk_dir(dir, &extensions, &cli.exclude, &mut all_matches, &mut file_count);
    apply_scope(cli, &mut all_matches);
    apply_severity(cli, &mut all_matches);

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
    Ok(())
}

fn walk_dir(
    dir: &Path,
    extensions: &[&str],
    exclude: &[String],
    matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>,
    count: &mut u32,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" || name == "vendor" || name == "dist" || name == "build" {
            continue;
        }
        let path_str = path.to_string_lossy();
        if exclude.iter().any(|pat| path_str.contains(pat.as_str())) {
            continue;
        }
        if path.is_dir() {
            walk_dir(&path, extensions, exclude, matches, count);
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_ascii_lowercase();
            if extensions.contains(&ext_str.as_str())
                && let Ok(code) = std::fs::read_to_string(&path)
            {
                let ft = if ext_str == "html" || ext_str == "htm" {
                    FileType::Html
                } else {
                    FileType::Js
                };
                let cfg = AnalyzerConfig::default();
                let file_matches = if ft == FileType::Html {
                    js_beautify_rs::analyzers::html_script_extractor::analyze_html(&code, Some(&path_str), &cfg)
                } else {
                    run_all(&code, Some(&path_str), ft, &cfg)
                };
                matches.extend(file_matches);
                *count += 1;
            }
        }
    }
}

fn apply_scope(cli: &Cli, matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>) {
    if cli.in_scope.is_none() && cli.out_of_scope.is_none() && cli.scope_file.is_none() {
        return;
    }
    let scope = js_beautify_rs::scope::ScopeMatcher::from_csv(
        cli.in_scope.as_deref().unwrap_or(""),
        cli.out_of_scope.as_deref().unwrap_or(""),
    );
    matches.retain(|m| {
        if m.match_kind == "hostname" || m.match_kind == "url" || m.match_kind == "comment_hostname" {
            scope.is_in_scope(&m.match_value)
        } else {
            true
        }
    });
}

fn apply_severity(cli: &Cli, matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>) {
    if let Some(ref min_sev_str) = cli.min_severity
        && let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::parse_label(min_sev_str)
    {
        matches.retain(|m| m.severity >= min_sev);
    }
}
