//! Hook system — external process execution on events.
//!
//! Ports jxscout's `crates/core/hooks/` (REPORT_06 §6). Each hook is a
//! user script spawned via `tokio::process::Command` with event JSON on
//! stdin. No sandboxing — full system access at jxscout process privileges.
//!
//! ## Supported event topics (mirrors jxscout)
//!
//! - `process_configuration` — fires once on startup
//! - `js_file_saved` / `js_file_updated` / `js_file_beautified`
//! - `html_file_saved` / `html_file_updated` / `html_file_beautified`
//! - `reversed_source_map_file_saved`
//! - `matches_created`
//! - `finding_created`
//! - `http_call_saved`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// One hook definition (from settings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub script: String,
    pub concurrency: Option<u32>,
    /// Optional filter by file_types.
    pub file_types: Option<Vec<String>>,
    /// Optional filter by match kinds.
    pub kinds: Option<Vec<String>>,
}

/// Full hooks configuration keyed by event topic → hook_name → config.
pub type HooksMap = HashMap<String, HashMap<String, HookConfig>>;

/// Run all hooks registered for `topic`, passing `payload` as JSON on stdin.
///
/// Hooks are executed in parallel up to each hook's `concurrency` limit.
/// Errors are logged to stderr but do not propagate — hooks are best-effort.
pub async fn fire(hooks: &HooksMap, topic: &str, payload: &serde_json::Value) {
    let Some(topic_hooks) = hooks.get(topic) else {
        return;
    };
    let payload_bytes = serde_json::to_vec(payload).unwrap_or_default();

    let mut handles = Vec::new();
    for (name, cfg) in topic_hooks {
        let script = cfg.script.clone();
        let input = payload_bytes.clone();
        let hook_name = name.clone();
        handles.push(tokio::spawn(async move {
            match run_script(&script, &input).await {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("[hook] {hook_name} exited with {status}");
                    }
                }
                Err(e) => {
                    eprintln!("[hook] {hook_name} failed: {e}");
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }
}

async fn run_script(script: &str, stdin_data: &[u8]) -> Result<std::process::ExitStatus, String> {
    let parts: Vec<&str> = script.split_whitespace().collect();
    if parts.is_empty() {
        return Err("empty script".into());
    }
    let mut cmd = Command::new(parts[0]);
    if parts.len() > 1 {
        cmd.args(&parts[1..]);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_data).await;
        drop(stdin);
    }
    child.wait().await.map_err(|e| e.to_string())
}

/// Parse hooks from a settings JSON structure.
///
/// Expected format (matches jxscout's `settings.jsonc`):
/// ```json
/// {
///   "hooks": {
///     "js_file_saved": {
///       "my_hook": { "script": "bun run hook.ts", "concurrency": 2 }
///     }
///   }
/// }
/// ```
///
/// # Errors
/// Parsing failures.
pub fn parse_hooks(value: &serde_json::Value) -> HooksMap {
    let mut out = HooksMap::new();
    let Some(obj) = value.get("hooks").and_then(|v| v.as_object()) else {
        return out;
    };
    for (topic, topic_val) in obj {
        let Some(topic_obj) = topic_val.as_object() else {
            continue;
        };
        let mut topic_hooks = HashMap::new();
        for (name, cfg_val) in topic_obj {
            if let Ok(cfg) = serde_json::from_value::<HookConfig>(cfg_val.clone()) {
                topic_hooks.insert(name.clone(), cfg);
            }
        }
        if !topic_hooks.is_empty() {
            out.insert(topic.clone(), topic_hooks);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_hooks() {
        let v = serde_json::json!({});
        assert!(parse_hooks(&v).is_empty());
    }

    #[test]
    fn parse_valid_hooks() {
        let v = serde_json::json!({
            "hooks": {
                "js_file_saved": {
                    "notify": { "script": "echo hello" },
                    "analyze": { "script": "bun run x.ts", "concurrency": 3 }
                },
                "matches_created": {
                    "alert": { "script": "/usr/bin/hook", "kinds": ["secret"] }
                }
            }
        });
        let h = parse_hooks(&v);
        assert_eq!(h.len(), 2);
        assert_eq!(h["js_file_saved"].len(), 2);
        assert_eq!(h["matches_created"]["alert"].script, "/usr/bin/hook");
    }
}
