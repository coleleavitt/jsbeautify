use super::Cli;
use js_beautify_rs::wordlist::extractors::html_extractor::{self as html_ext, HtmlExtractConfig};
use js_beautify_rs::wordlist::extractors::js_ast_extractor::{self, ExtractConfig};
use std::fs;

pub(super) fn run_wordlist(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let min_len = cli.min_length.unwrap_or(2);
    let max_len = cli.max_length.unwrap_or(0);
    let limit = cli.limit.unwrap_or(0);
    let mut all_words: Vec<(String, &str)> = Vec::new();
    let is_html = code.trim_start().starts_with('<')
        || std::path::Path::new(&cli.input)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| {
                ["html", "htm"]
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            });

    if is_html {
        for w in html_ext::extract_sourced(code, &HtmlExtractConfig::default()) {
            all_words.push((w.word, w.source));
        }
        for block in js_beautify_rs::analyzers::html_script_extractor::extract_scripts(code) {
            for w in js_ast_extractor::extract_sourced(&block.content, &ExtractConfig::default()) {
                all_words.push((w.word, w.source));
            }
        }
    } else {
        for w in js_ast_extractor::extract_sourced(code, &ExtractConfig::default()) {
            all_words.push((w.word, w.source));
        }
    }

    if !cli.sources.is_empty() {
        all_words.retain(|(_, src)| cli.sources.iter().any(|s| s == *src));
    }
    all_words.retain(|(w, _)| w.len() >= min_len && (max_len == 0 || w.len() <= max_len));
    if cli.lowercase {
        for (w, _) in &mut all_words {
            *w = w.to_ascii_lowercase();
        }
    }

    let mut seen = std::collections::HashSet::new();
    all_words.retain(|(w, _)| seen.insert(w.clone()));
    if cli.plurals {
        let mut extras: Vec<(String, &str)> = Vec::new();
        for (w, src) in &all_words {
            if w.ends_with('s') {
                if w.len() > 3 {
                    extras.push((w[..w.len() - 1].to_string(), *src));
                }
            } else {
                extras.push((format!("{w}s"), *src));
            }
        }
        for e in extras {
            if seen.insert(e.0.clone()) {
                all_words.push(e);
            }
        }
    }

    all_words.sort_unstable_by(|a, b| a.0.cmp(&b.0));
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
    Ok(())
}
