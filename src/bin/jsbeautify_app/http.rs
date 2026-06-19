use super::Cli;
use std::time::Duration;

pub(super) fn run_repeater(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config = repeater_config(cli);
    let result = js_beautify_rs::repeater::replay(code, &config)?;
    let output_text = if cli.json {
        serde_json::to_string_pretty(&result)?
    } else {
        format!(
            "HTTP {} | {} bytes | {}ms\n{}",
            result.status_code, result.body_size, result.elapsed_ms, result.body
        )
    };
    output(cli, &output_text)
}

pub(super) fn run_fuzz(cli: &Cli, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    use js_beautify_rs::fuzz::{AttackMode, apply_payloads, generate_combinations, load_wordlist};

    let mode = cli
        .attack_mode
        .as_deref()
        .and_then(|mode| mode.parse().ok())
        .unwrap_or(AttackMode::Sniper);
    let markers: Vec<String> = if cli.marker.is_empty() {
        vec!["FUZZ".into(), "FUZZ2".into(), "FUZZ3".into()]
    } else {
        cli.marker.clone()
    };

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
    let delay = cli.delay_ms.unwrap_or(0);
    let match_statuses: Vec<u16> = cli
        .match_status
        .as_deref()
        .map(|s| s.split(',').filter_map(|c| c.trim().parse().ok()).collect())
        .unwrap_or_default();
    let config = repeater_config(cli);

    eprintln!(
        "[FUZZ] Mode: {mode}, Combinations: {}, Concurrency: {}",
        combinations.len(),
        cli.concurrency.unwrap_or(10)
    );

    let mut results: Vec<js_beautify_rs::fuzz::FuzzResult> = Vec::new();
    for (idx, combo) in combinations.iter().enumerate() {
        let rendered = apply_payloads(code, &markers, combo);
        match js_beautify_rs::repeater::replay(&rendered, &config) {
            Ok(resp) => push_fuzz_result(cli, &match_statuses, &mut results, idx, combo, &resp),
            Err(e) if !cli.json => eprintln!("[{idx}] {combo:?} → ERROR: {e}"),
            Err(_) => {}
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
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
    Ok(())
}

fn repeater_config(cli: &Cli) -> js_beautify_rs::repeater::RepeaterConfig {
    js_beautify_rs::repeater::RepeaterConfig {
        url: cli.repeat_url.clone(),
        proxy_host: cli.proxy_host.clone(),
        proxy_port: cli.proxy_port,
        update_content_length: true,
        timeout_secs: 30,
    }
}

fn push_fuzz_result(
    cli: &Cli,
    match_statuses: &[u16],
    results: &mut Vec<js_beautify_rs::fuzz::FuzzResult>,
    idx: usize,
    combo: &[String],
    resp: &js_beautify_rs::repeater::RepeaterResult,
) {
    if !match_statuses.is_empty() && !match_statuses.contains(&resp.status_code) {
        return;
    }
    if let Some(ref filter) = cli.filter_body
        && resp.body.contains(filter.as_str())
    {
        return;
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
    results.push(js_beautify_rs::fuzz::FuzzResult {
        index: idx,
        payloads: combo.to_vec(),
        status_code: resp.status_code,
        response_size: resp.body_size,
        body_preview: preview,
    });
}

fn output(cli: &Cli, output_text: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(out_path) = &cli.analysis_out {
        std::fs::write(out_path, output_text)?;
    } else {
        println!("{output_text}");
    }
    Ok(())
}
