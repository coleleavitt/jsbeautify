use super::Cli;
use js_beautify_rs::analyzers::match_kind::FileType;
use js_beautify_rs::analyzers::{AnalyzerConfig, run_all};
use std::fs;

pub(super) const fn is_analysis_requested(cli: &Cli) -> bool {
    cli.analyze
        || cli.analyze_all
        || cli.scan_secrets
        || cli.find_sinks
        || cli.extract_paths
        || cli.extract_hostnames
        || cli.extract_graphql
        || cli.extract_packages
        || cli.find_sensitive_props
}

pub(super) fn run_analysis(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
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
            api_calls: true,
            entropy_secrets: cli.scan_secrets,
            taint: cli.find_sinks,
            idor: cli.extract_paths || cli.find_sinks,
            proto_pollution: cli.find_sinks,
            resolve_endpoints: cli.extract_paths,
        }
    };

    let file_path = (cli.input != "-").then_some(cli.input.as_str());
    let mut matches = run_all(code, file_path, FileType::Js, &cfg);

    if cli.extract_hostnames && !cli.extract_paths && !cli.analyze && !cli.analyze_all {
        matches.retain(|m| matches!(m.match_kind.as_str(), "hostname" | "comment_hostname"));
    }
    apply_scope(cli, &mut matches);
    apply_derived(cli, &mut matches);
    apply_severity(cli, &mut matches);
    if cli.unique {
        matches = js_beautify_rs::analyzers::dedup_unique_values(matches);
    }

    if cli.wordlist_from_matches {
        return output_wordlist_from_matches(cli, &matches);
    }
    if cli.sarif {
        return output_sarif(cli, &matches);
    }

    let output_text = if cli.json {
        serde_json::to_string_pretty(&serde_json::json!({
            "findings": matches,
        }))?
    } else {
        text_report(&matches)
    };

    if let Some(out_path) = &cli.analysis_out {
        fs::write(out_path, &output_text)?;
        eprintln!("Analysis written to {}", out_path.display());
    } else {
        println!("{output_text}");
    }
    Ok(())
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

fn apply_derived(cli: &Cli, matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>) {
    for spec in &cli.derive {
        let parts: Vec<&str> = spec.splitn(3, ':').collect();
        if parts.len() >= 2 {
            let include = if parts.len() == 3 { vec![parts[2]] } else { vec![] };
            if let Ok(derived) =
                js_beautify_rs::analyzers::derived::DerivedAnalyzer::new(parts[1], parts[0], &include, &[])
            {
                matches.extend(derived.apply(matches));
            }
        }
    }
}

fn apply_severity(cli: &Cli, matches: &mut Vec<js_beautify_rs::analyzers::AnalyzerMatch>) {
    if let Some(ref min_sev_str) = cli.min_severity
        && let Some(min_sev) = js_beautify_rs::analyzers::regex_bank::Severity::parse_label(min_sev_str)
    {
        matches.retain(|m| m.severity >= min_sev);
    }
}

fn output_wordlist_from_matches(
    cli: &Cli,
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
) -> Result<(), Box<dyn std::error::Error>> {
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
                .split(['/', '?', '=', '&', '#', ':', '-', '_', '.'])
                .filter(|s| s.len() > 1)
                .map(str::to_ascii_lowercase)
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
    Ok(())
}

fn output_sarif(
    cli: &Cli,
    matches: &[js_beautify_rs::analyzers::AnalyzerMatch],
) -> Result<(), Box<dyn std::error::Error>> {
    let sarif = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "jsbeautify", "version": env!("CARGO_PKG_VERSION") } },
            "results": matches.iter().map(|m| serde_json::json!({
                "ruleId": m.match_kind,
                "message": { "text": m.match_value },
                "locations": [{ "physicalLocation": { "artifactLocation": { "uri": m.file_path.as_deref().unwrap_or("stdin") }, "region": { "startLine": m.position.start.line, "startColumn": m.position.start.column + 1, "endLine": m.position.end.line, "endColumn": m.position.end.column + 1 } } }]
            })).collect::<Vec<_>>()
        }]
    });
    let output_text = serde_json::to_string_pretty(&sarif)?;
    if let Some(out_path) = &cli.analysis_out {
        fs::write(out_path, &output_text)?;
    } else {
        println!("{output_text}");
    }
    Ok(())
}

fn text_report(matches: &[js_beautify_rs::analyzers::AnalyzerMatch]) -> String {
    let mut lines = Vec::with_capacity(matches.len());
    for m in matches {
        lines.push(format!(
            "[{}] {} (line {}:{})",
            m.match_kind, m.match_value, m.position.start.line, m.position.start.column
        ));
    }
    if lines.is_empty() {
        lines.push("No findings.".to_string());
    }
    lines.join("\n")
}
