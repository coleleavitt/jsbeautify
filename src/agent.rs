//! Agentic analysis loop — the "recon brain" reasoning tier.
//!
//! Static analyzers find leads; the [`chains`](crate::chains) engine correlates
//! them; [`validate`](crate::validate) confirms them. This module ties those
//! into an **LLM-driven investigation loop** (ReAct / ADK-style): the model is
//! given a set of *tools*, and on each turn it either calls a tool to gather
//! more evidence or emits a final report. The loop is what lets the system
//! *chain* a vulnerability — pull one thread (a leaked token), call a tool to
//! see what it unlocks (an IDOR endpoint on the same host), and keep going until
//! it has an end-to-end attack path — the same way a human researcher loops
//! search → read → pivot.
//!
//! Design for testability: the model is abstracted behind [`ChatBackend`] so the
//! whole loop runs deterministically against a scripted mock in unit tests; the
//! real backend ([`crate::llm::LlmClient`]) is only used behind the CLI flag.
//! Tool dispatch is pure data-in/data-out over the finding set + chains.
#![allow(clippy::doc_markdown)] // prose names tools/CWE ids/brands

use serde::{Deserialize, Serialize};

use crate::analyzers::match_kind::AnalyzerMatch;
use crate::chains::{self, AttackChain};

/// Anything that can answer a chat turn. Implemented by the real LLM client and
/// by the test mock.
pub trait ChatBackend {
    /// Given the system prompt + running transcript, return the assistant's next
    /// message (expected to be a JSON action).
    ///
    /// # Errors
    /// Implementation-defined backend errors (e.g. transport/decoding for a real LLM).
    fn respond(&mut self, system: &str, transcript: &str) -> Result<String, String>;
}

/// The toolbox the agent investigates with. Holds the evidence (findings +
/// derived chains) and answers tool calls over it.
pub struct Investigation<'a> {
    findings: &'a [AnalyzerMatch],
    chains: Vec<AttackChain>,
    /// Optional source the agent can read windows of.
    source: Option<&'a str>,
}

impl<'a> Investigation<'a> {
    #[must_use]
    pub fn new(findings: &'a [AnalyzerMatch], source: Option<&'a str>) -> Self {
        Self {
            findings,
            chains: chains::correlate(findings),
            source,
        }
    }

    /// JSON description of the available tools, embedded in the system prompt.
    const fn tool_catalog() -> &'static str {
        r#"[
  {"tool":"list_findings","args":{},"desc":"Return all static findings (kind, value, severity, cwe)."},
  {"tool":"get_chains","args":{},"desc":"Return correlated multi-step attack chains."},
  {"tool":"read_source","args":{"offset":"int","len":"int"},"desc":"Read a window of the analyzed source."},
  {"tool":"finish","args":{"report":"string"},"desc":"Emit the final investigation report and stop."}
]"#
    }

    /// Execute one tool call and return its JSON result.
    fn dispatch(&self, call: &ToolCall) -> String {
        match call.tool.as_str() {
            "list_findings" => {
                let items: Vec<_> = self
                    .findings
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "kind": m.match_kind,
                            "value": truncate(&m.match_value, 120),
                            "severity": format!("{:?}", m.severity),
                            "cwe": m.metadata.as_ref().and_then(|v| v.get("cwe")),
                        })
                    })
                    .collect();
                serde_json::json!({ "findings": items }).to_string()
            }
            "get_chains" => serde_json::to_string(&self.chains).unwrap_or_else(|_| "[]".to_string()),
            "read_source" => {
                let Some(src) = self.source else {
                    return r#"{"error":"no source available"}"#.to_string();
                };
                let offset = call.args.get("offset").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
                let len = call.args.get("len").and_then(serde_json::Value::as_u64).unwrap_or(400) as usize;
                let window = source_window(src, offset, len);
                serde_json::json!({ "source": window }).to_string()
            }
            other => serde_json::json!({ "error": format!("unknown tool {other}") }).to_string(),
        }
    }
}

/// A tool call parsed from the model's reply.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Result of an agent investigation.
#[derive(Debug, Clone, Serialize)]
pub struct AgentReport {
    pub report: String,
    pub turns: usize,
    /// The chains the agent had available (also surfaced standalone).
    pub chains: Vec<AttackChain>,
}

/// Parse the model's reply into a tool call. Tolerates a JSON object embedded in
/// prose / code fences.
fn parse_tool_call(reply: &str) -> Option<ToolCall> {
    let (start, end) = (reply.find('{')?, reply.rfind('}')?);
    if end <= start {
        return None;
    }
    serde_json::from_str::<ToolCall>(&reply[start..=end]).ok()
}

/// Run the investigation loop. The agent calls tools until it issues `finish` or
/// `max_turns` is reached.
///
/// # Errors
/// Propagates backend errors.
pub fn investigate(
    backend: &mut dyn ChatBackend,
    investigation: &Investigation<'_>,
    max_turns: usize,
) -> Result<AgentReport, String> {
    use std::fmt::Write as _;
    let system = format!(
        "You are a web-security analysis agent investigating a JavaScript target. \
         Chain individual findings into concrete attack paths. Each turn, reply with \
         EXACTLY ONE JSON tool call from this catalog and nothing else:\n{}\n\
         Start by listing findings and chains, read source for any finding you want to \
         confirm, then call finish with a concise report of the strongest attack chain(s).",
        Investigation::tool_catalog()
    );

    let mut transcript = String::new();
    for turn in 1..=max_turns {
        let reply = backend.respond(&system, &transcript)?;
        let Some(call) = parse_tool_call(&reply) else {
            // Treat an unparseable reply as the final report (graceful exit).
            return Ok(AgentReport {
                report: reply,
                turns: turn,
                chains: investigation.chains.clone(),
            });
        };
        if call.tool == "finish" {
            let report = call
                .args
                .get("report")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(no report)")
                .to_string();
            return Ok(AgentReport {
                report,
                turns: turn,
                chains: investigation.chains.clone(),
            });
        }
        let result = investigation.dispatch(&call);
        write!(transcript, "\nTOOL {} -> {}\n", call.tool, truncate(&result, 4000)).ok();
    }
    Ok(AgentReport {
        report: "investigation hit max turns without finishing".to_string(),
        turns: max_turns,
        chains: investigation.chains.clone(),
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut end = n;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

fn source_window(src: &str, offset: usize, len: usize) -> String {
    let offset = offset.min(src.len());
    let mut start = offset;
    while start > 0 && !src.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (offset + len).min(src.len());
    while end < src.len() && !src.is_char_boundary(end) {
        end += 1;
    }
    src[start..end].to_string()
}

/// Adapter so [`crate::llm::LlmClient`] can drive the loop.
impl ChatBackend for crate::llm::LlmClient {
    /// # Errors
    /// Propagates the underlying chat transport/decoding error.
    fn respond(&mut self, system: &str, transcript: &str) -> Result<String, String> {
        let user = if transcript.is_empty() {
            "Begin the investigation.".to_string()
        } else {
            format!("Transcript so far:\n{transcript}\n\nNext tool call:")
        };
        self.chat(system, &user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::match_kind::{FileType, PositionRange, Provenance};
    use crate::analyzers::regex_bank::Severity;

    fn mk(kind: &str, value: &str, sev: Severity) -> AnalyzerMatch {
        AnalyzerMatch::new(
            kind,
            value,
            PositionRange::new(1, 0, 1, 1),
            FileType::Js,
            Some("https://t.com/a.js".into()),
        )
        .with_provenance(Provenance::Ast)
        .with_severity(sev)
    }

    /// A scripted backend that replays a fixed list of replies in order.
    struct ScriptedBackend {
        replies: Vec<String>,
        i: usize,
        seen_tools: Vec<String>,
    }
    impl ChatBackend for ScriptedBackend {
        fn respond(&mut self, _system: &str, transcript: &str) -> Result<String, String> {
            // Record which tool results have appeared so the test can assert flow.
            if let Some(last) = transcript.lines().last()
                && last.starts_with("TOOL ")
            {
                self.seen_tools.push(last.to_string());
            }
            let r = self
                .replies
                .get(self.i)
                .cloned()
                .unwrap_or_else(|| r#"{"tool":"finish","args":{"report":"done"}}"#.to_string());
            self.i += 1;
            Ok(r)
        }
    }

    #[test]
    fn loop_calls_tools_then_finishes() {
        let findings = vec![
            mk("open_redirect", "location.href=u", Severity::Medium),
            mk("high_entropy_secret", "aB3xZ9qLpW...", Severity::High),
        ];
        let inv = Investigation::new(&findings, Some("var x = location.hash;"));
        let mut backend = ScriptedBackend {
            replies: vec![
                r#"{"tool":"list_findings","args":{}}"#.to_string(),
                r#"{"tool":"get_chains","args":{}}"#.to_string(),
                r#"{"tool":"finish","args":{"report":"account_takeover via redirect + secret"}}"#.to_string(),
            ],
            i: 0,
            seen_tools: Vec::new(),
        };
        let report = investigate(&mut backend, &inv, 10).unwrap();
        assert_eq!(report.turns, 3);
        assert!(report.report.contains("account_takeover"));
        // The agent had the correlated chain available.
        assert!(report.chains.iter().any(|c| c.kind == "account_takeover"));
    }

    #[test]
    fn parse_tool_call_tolerates_prose() {
        let c = parse_tool_call("Sure! {\"tool\":\"list_findings\",\"args\":{}} done").unwrap();
        assert_eq!(c.tool, "list_findings");
    }

    #[test]
    fn unparseable_reply_becomes_report() {
        let findings = vec![mk("hostname", "a.com", Severity::Info)];
        let inv = Investigation::new(&findings, None);
        let mut backend = ScriptedBackend {
            replies: vec!["no json here, just a summary".to_string()],
            i: 0,
            seen_tools: Vec::new(),
        };
        let report = investigate(&mut backend, &inv, 5).unwrap();
        assert!(report.report.contains("summary"));
    }

    #[test]
    fn dispatch_list_findings_returns_kinds() {
        let findings = vec![mk("dom_xss", "el.innerHTML=u", Severity::High)];
        let inv = Investigation::new(&findings, None);
        let out = inv.dispatch(&ToolCall {
            tool: "list_findings".to_string(),
            args: serde_json::json!({}),
        });
        assert!(out.contains("dom_xss"));
    }
}
