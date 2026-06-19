use super::Cli;
use std::fs;

pub(super) fn run_serve(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
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
        .map_err(std::convert::Into::into)
}

pub(super) fn run_crtsh(cli: &Cli, domain: &str) -> Result<(), Box<dyn std::error::Error>> {
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

pub(super) fn run_recon(cli: &Cli, domain: &str) -> Result<(), Box<dyn std::error::Error>> {
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
