use super::Cli;
use js_beautify_rs::analyzers::match_kind::FileType;
use js_beautify_rs::analyzers::{AnalyzerConfig, run_all};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

pub(super) fn run_reconstruct_sourcemap(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let Some(out_dir) = cli.reconstruct_sourcemap.clone() else {
        return Ok(());
    };
    let map_json = if cli.input == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(&cli.input)?
    };
    let dir = std::path::Path::new(&out_dir);
    let written = js_beautify_rs::sourcemap::reconstruct_tree(&map_json, dir)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    eprintln!(
        "Reconstructed {} source file(s) into {}/reversed/",
        written.len(),
        dir.display()
    );
    for w in &written {
        println!("{w}");
    }
    Ok(())
}

pub(super) fn run_relationships(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}

pub(super) fn run_reverse_sourcemap(cli: &Cli, map_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let map_json = fs::read_to_string(map_path).map_err(|e| format!("failed to read {}: {}", map_path.display(), e))?;
    let reversed =
        js_beautify_rs::sourcemap::reverse(&map_json).map_err(|e| format!("failed to parse sourcemap: {e}"))?;

    if let Some(ref dir) = cli.reversed_sources_dir {
        fs::create_dir_all(dir)?;
        for rs in &reversed {
            let safe = rs.source_path.replace("..", "_").trim_start_matches('/').to_string();
            let dest = dir.join(&safe);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&dest, &rs.content)?;
        }
        eprintln!("Wrote {} reversed source(s) to {}", reversed.len(), dir.display());
    }

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
    if let Some(ref min_sev_str) = cli.min_severity
        && let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::parse_label(min_sev_str)
    {
        all_matches.retain(|m| m.severity >= min_sev);
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
    Ok(())
}
