//! Reusable OpenAI-compatible chat client.
//!
//! One place to talk to any `/v1/chat/completions` endpoint — OpenAI,
//! OpenRouter, Ollama, llama.cpp, vLLM, Azure OpenAI, … — used by the LLM
//! renaming tier.
//!
//! ## Configuration precedence
//! Explicit CLI flags > project env > standard OpenAI env > defaults:
//! - URL:   `--llm-url` › `JSBEAUTIFY_LLM_URL` › `OPENAI_BASE_URL` (+ `/chat/completions`) › the default OpenAI endpoint
//! - Model: `--llm-model` › `JSBEAUTIFY_LLM_MODEL` › `OPENAI_MODEL` › `gpt-4o-mini`
//! - Key:   `--llm-key` › `JSBEAUTIFY_LLM_KEY` › `OPENAI_API_KEY`
//!
//! A client is only usable when *some* endpoint is resolvable. With the stock
//! OpenAI default URL we additionally require a key, so `OPENAI_API_KEY` alone
//! is enough to "just work".
#![allow(clippy::doc_markdown)] // module docs name brands (OpenAI, OpenRouter, …)

use std::time::Duration;

use serde::Deserialize;

const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Explicit overrides, typically from CLI flags. Any `None` falls back to env.
#[derive(Debug, Clone, Default)]
pub struct LlmOverrides {
    pub url: Option<String>,
    pub model: Option<String>,
    pub key: Option<String>,
}

/// Resolved configuration for an OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub url: String,
    pub model: String,
    pub key: Option<String>,
}

impl LlmConfig {
    /// Resolve config from explicit overrides, then env, then defaults.
    ///
    /// Returns `None` when no usable endpoint can be determined (e.g. the
    /// default OpenAI URL with no `OPENAI_API_KEY`).
    #[must_use]
    pub fn resolve(ov: &LlmOverrides) -> Option<Self> {
        Self::resolve_from(ov, |k| std::env::var(k).ok())
    }

    /// Testable core of [`resolve`](Self::resolve): env lookups go through `env`.
    fn resolve_from(ov: &LlmOverrides, env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let explicit_url = ov.url.clone().or_else(|| env("JSBEAUTIFY_LLM_URL"));
        // OPENAI_BASE_URL is a base like https://api.openai.com/v1 — append the path.
        let openai_base = env("OPENAI_BASE_URL").map(|b| {
            let b = b.trim_end_matches('/');
            if b.ends_with("/chat/completions") {
                b.to_string()
            } else {
                format!("{b}/chat/completions")
            }
        });
        let is_default_url = explicit_url.is_none() && openai_base.is_none();
        let url = explicit_url
            .or(openai_base)
            .unwrap_or_else(|| DEFAULT_OPENAI_URL.to_string());

        let model = ov
            .model
            .clone()
            .or_else(|| env("JSBEAUTIFY_LLM_MODEL"))
            .or_else(|| env("OPENAI_MODEL"))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let key = ov
            .key
            .clone()
            .or_else(|| env("JSBEAUTIFY_LLM_KEY"))
            .or_else(|| env("OPENAI_API_KEY"));

        // Talking to the stock OpenAI endpoint without a key can't work.
        if is_default_url && key.is_none() {
            return None;
        }
        Some(Self { url, model, key })
    }
}

/// Blocking OpenAI-compatible chat client.
pub struct LlmClient {
    config: LlmConfig,
    client: reqwest::blocking::Client,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

impl LlmClient {
    /// Build a client from resolved config.
    ///
    /// # Errors
    /// Fails if the underlying HTTP client cannot be constructed.
    pub fn new(config: LlmConfig) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("build llm http client: {e}"))?;
        Ok(Self { config, client })
    }

    /// Resolve config from overrides+env and build a client, or `None` if no
    /// endpoint is configured.
    #[must_use]
    pub fn from_overrides(ov: &LlmOverrides) -> Option<Self> {
        let config = LlmConfig::resolve(ov)?;
        Self::new(config).ok()
    }

    /// The resolved model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Send a single system+user chat turn at temperature 0, returning the
    /// assistant message content.
    ///
    /// # Errors
    /// Network errors, non-2xx responses, or an empty/!parseable body.
    pub fn chat(&self, system: &str, user: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.config.model,
            "temperature": 0,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        let mut req = self.client.post(&self.config.url).json(&body);
        if let Some(key) = &self.config.key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().map_err(|e| format!("llm request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("llm endpoint returned HTTP {}", resp.status()));
        }
        let parsed: ChatResponse = resp.json().map_err(|e| format!("llm decode: {e}"))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| "llm response had no choices".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn openai_api_key_alone_is_enough() {
        let cfg = LlmConfig::resolve_from(&LlmOverrides::default(), env_map(&[("OPENAI_API_KEY", "sk-test")])).unwrap();
        assert_eq!(cfg.url, DEFAULT_OPENAI_URL);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert_eq!(cfg.key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn default_url_without_key_is_unusable() {
        assert!(LlmConfig::resolve_from(&LlmOverrides::default(), env_map(&[])).is_none());
    }

    #[test]
    fn openai_base_url_gets_chat_completions_appended() {
        let cfg = LlmConfig::resolve_from(
            &LlmOverrides::default(),
            env_map(&[
                ("OPENAI_BASE_URL", "https://oai.example.com/v1"),
                ("OPENAI_API_KEY", "k"),
            ]),
        )
        .unwrap();
        assert_eq!(cfg.url, "https://oai.example.com/v1/chat/completions");
    }

    #[test]
    fn local_endpoint_needs_no_key() {
        // A non-default URL (e.g. Ollama) is usable without any key.
        let cfg = LlmConfig::resolve_from(
            &LlmOverrides::default(),
            env_map(&[("JSBEAUTIFY_LLM_URL", "http://localhost:11434/v1/chat/completions")]),
        )
        .unwrap();
        assert_eq!(cfg.url, "http://localhost:11434/v1/chat/completions");
        assert!(cfg.key.is_none());
    }

    #[test]
    fn explicit_overrides_beat_env() {
        let ov = LlmOverrides {
            url: Some("http://flag/v1/chat/completions".to_string()),
            model: Some("flag-model".to_string()),
            key: Some("flag-key".to_string()),
        };
        let cfg = LlmConfig::resolve_from(
            &ov,
            env_map(&[
                ("JSBEAUTIFY_LLM_URL", "http://env/x"),
                ("OPENAI_MODEL", "env-model"),
                ("OPENAI_API_KEY", "env-key"),
            ]),
        )
        .unwrap();
        assert_eq!(cfg.url, "http://flag/v1/chat/completions");
        assert_eq!(cfg.model, "flag-model");
        assert_eq!(cfg.key.as_deref(), Some("flag-key"));
    }

    #[test]
    fn project_env_beats_openai_env() {
        let cfg = LlmConfig::resolve_from(
            &LlmOverrides::default(),
            env_map(&[
                ("JSBEAUTIFY_LLM_MODEL", "proj-model"),
                ("OPENAI_MODEL", "oai-model"),
                ("OPENAI_API_KEY", "k"),
            ]),
        )
        .unwrap();
        assert_eq!(cfg.model, "proj-model");
    }
}
