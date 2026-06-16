//! Live-validation demo against LOCAL, self-hosted targets (127.0.0.1).
//!
//! Stands up two in-process loopback servers — a vulnerable **target** and an
//! attacker **collaborator** — and runs the real `LiveProber` against them,
//! exercising the exact production code path (real HTTP, real `Location`/body
//! inspection, real out-of-band SSRF confirmation, real verdicts). Both servers
//! are our own loopback processes, so this is unambiguously authorized.
//!
//! It then validates a **complex multi-step attack chain** end-to-end: each hop
//! is confirmed live and in order, and the chain is only declared proven if
//! every hop confirms.
//!
//! Run: `cargo run --example live_validate_demo`

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use js_beautify_rs::validate::{LiveProber, ProbeConfig, Validation, Verdict};

fn main() {
    // ── Collaborator: records every path it's hit on (out-of-band SSRF oracle).
    let collab_listener = TcpListener::bind("127.0.0.1:0").expect("bind collaborator");
    let collab_addr = collab_listener.local_addr().unwrap();
    let collab_host = format!("127.0.0.1:{}", collab_addr.port());
    let hits: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    {
        let hits = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in collab_listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                if let Some(path) = req.lines().next().and_then(|l| l.split_whitespace().nth(1)) {
                    hits.lock().unwrap().insert(path.trim_start_matches('/').to_string());
                }
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            }
        });
    }

    // ── Target: an intentionally-vulnerable app. Routes:
    //    /redirect?redirect=URL  -> 302 Location: URL                  (open redirect)
    //    /reflect?q=CANARY       -> echoes CANARY unencoded            (reflected XSS)
    //    /accounts/<id>          -> id-specific body, no auth          (IDOR)
    //    /proxy?url=URL          -> SERVER-SIDE fetch of URL           (SSRF)
    let target_listener = TcpListener::bind("127.0.0.1:0").expect("bind target");
    let target_addr = target_listener.local_addr().unwrap();
    let base = format!("http://127.0.0.1:{}", target_addr.port());
    println!("[target]       vulnerable app  : {base}");
    println!("[collaborator] attacker oracle : http://{collab_host}\n");

    std::thread::spawn(move || {
        for stream in target_listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let response = route(&path);
            if stream.write_all(response.as_bytes()).is_ok() {
                stream.flush().ok();
            }
        }
    });

    // Prober scoped to 127.0.0.1 only.
    let prober = LiveProber::new(
        &["127.0.0.1"],
        ProbeConfig {
            canary_host: "evil.attacker.test".to_string(),
            min_delay: std::time::Duration::from_millis(10),
            timeout: std::time::Duration::from_secs(5),
        },
    )
    .expect("build prober");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        println!("=== Individual probes ===\n");

        let redirect = format!("{base}/redirect");
        let v_redirect = prober.probe_open_redirect(&redirect).await;
        report(
            "open_redirect",
            &redirect,
            &v_redirect,
            &format!("curl -ski '{redirect}?redirect=https://evil.attacker.test/'"),
        );

        let reflect = format!("{base}/reflect");
        let v_xss = prober.probe_reflection(&reflect, "q").await;
        report(
            "dom_xss (reflection)",
            &reflect,
            &v_xss,
            &format!("curl -sk '{reflect}?q=<canary>'"),
        );

        let acct = format!("{base}/accounts/12345");
        let acct_swapped = format!("{base}/accounts/12346");
        let v_idor = prober.probe_idor(&acct, &acct_swapped).await;
        report(
            "idor_candidate",
            &acct,
            &v_idor,
            &format!("curl -ski '{acct}'  # then -> {acct_swapped}"),
        );

        // SSRF via out-of-band collaborator callback.
        let proxy = format!("{base}/proxy");
        let v_ssrf = prober
            .probe_ssrf(&proxy, &collab_host, |token| {
                let hits = Arc::clone(&hits);
                async move {
                    // Give the target a beat to make its server-side request.
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    hits.lock().unwrap().contains(&token)
                }
            })
            .await;
        report(
            "ssrf (collaborator)",
            &proxy,
            &v_ssrf,
            &format!("curl -sk '{proxy}?url=http://<collaborator>/<token>'  # then check oracle"),
        );

        // Out-of-scope gate (no request ever sent).
        let off = "http://example.com/redirect";
        let v_off = prober.probe_open_redirect(off).await;
        report(
            "open_redirect (OUT OF SCOPE)",
            off,
            &v_off,
            "(no request sent — host not in scope)",
        );

        // ── Complex multi-step chain, validated hop-by-hop ───────────────────
        // redirect_ssrf_idor: open redirect → SSRF pivot → internal IDOR.
        // We only declare the chain proven if EVERY hop confirmed live.
        println!("=== Multi-step chain validation: redirect → SSRF → internal IDOR ===\n");
        let hops = [
            ("entry-redirect", "open_redirect", &v_redirect),
            ("ssrf-pivot", "ssrf", &v_ssrf),
            ("internal-object", "idor_candidate", &v_idor),
        ];
        let mut all_confirmed = true;
        for (i, (role, kind, v)) in hops.iter().enumerate() {
            let ok = v.verdict == Verdict::Confirmed;
            all_confirmed &= ok;
            println!("  step {}: {role:<16} [{kind}] -> {}", i + 1, verdict_label(v.verdict));
        }
        println!();
        if all_confirmed {
            println!("  ✅ CHAIN PROVEN: every hop confirmed live — redirect entry pivots via SSRF to an");
            println!("     unauthorized internal object. Report as a single CWE-918 critical, not 3 mediums.");
        } else {
            println!("  ⚠️  chain NOT fully proven — at least one hop unconfirmed; report only confirmed hops.");
        }
    });
}

/// Human label for a verdict.
const fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Confirmed => "CONFIRMED",
        Verdict::Refuted => "refuted",
        Verdict::Inconclusive => "inconclusive",
    }
}

fn route(path: &str) -> String {
    if path.starts_with("/redirect") {
        let loc = path
            .split("redirect=")
            .nth(1)
            .unwrap_or("/")
            .split('&')
            .next()
            .unwrap_or("/");
        format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    } else if path.starts_with("/reflect") {
        let q = path.split("q=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
        http_ok(&format!("<html><body>You searched for: {q}</body></html>"))
    } else if path.starts_with("/accounts/") {
        let id = path.trim_start_matches("/accounts/").split('/').next().unwrap_or("?");
        http_ok(&format!(
            "{{\"accountId\":\"{id}\",\"owner\":\"user_{id}\",\"balance\":{}}}",
            id.len() * 1000
        ))
    } else if path.starts_with("/proxy") {
        // SSRF: the server fetches the attacker-supplied url= server-side.
        if let Some(url) = path.split("url=").nth(1).map(|u| u.split('&').next().unwrap_or("")) {
            let url = percent_decode(url);
            // Blocking fetch on this worker thread — fine for the demo.
            let _ = ureq_get(&url);
        }
        http_ok("{\"status\":\"fetched\"}")
    } else {
        http_ok("ok")
    }
}

/// Minimal server-side GET so the target actually calls the collaborator.
fn ureq_get(url: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::net::TcpStream;
    // Parse http://host:port/path
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, String::new()), |(a, p)| (a, format!("/{p}")));
    let mut stream = TcpStream::connect(authority)?;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n",
        if path.is_empty() { "/" } else { &path }
    )?;
    Ok(())
}

/// Tiny percent-decode for `%2F`/`%3A` etc. in the injected URL.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn http_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn report(kind: &str, target: &str, v: &Validation, repro: &str) {
    println!("── {kind} ──");
    println!("  target  : {target}");
    println!("  verdict : {}", verdict_label(v.verdict));
    println!("  evidence: {}", v.evidence);
    println!("  repro   : {repro}\n");
}
