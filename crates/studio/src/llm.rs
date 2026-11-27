//! The LLM port, and its two adapters: any OpenAI-compatible server (OpenAI, Ollama,
//! OpenRouter, Groq, LM Studio…) and Anthropic's Messages API.

use std::time::Duration;

use serde_json::{Value, json};

use crate::b64;
use crate::{Error, Result};

/// A piece of what the LLM is shown.
#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    /// A PNG picture (frames of a moment).
    Png(Vec<u8>),
}

pub trait Llm: Send + Sync {
    /// One question, one answer.
    fn ask(&self, system: &str, parts: &[Part]) -> Result<String>;
    /// For messages: "anthropic claude-sonnet-5-5".
    fn name(&self) -> String;
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn post(
    req: ureq::RequestBuilder<ureq::typestate::WithBody>,
    body: Value,
    name: &str,
) -> Result<Value> {
    let mut res = req.send_json(body).map_err(|e| Error::Llm {
        name: name.to_owned(),
        message: format!("isn't answering: {e}"),
    })?;
    let status = res.status();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| text.chars().take(300).collect());
        let hint = match status.as_u16() {
            401 | 403 => " (check the API key)",
            404 => " (check the model name and the base URL)",
            429 => " (rate limited: wait a bit)",
            _ => "",
        };
        return Err(Error::Llm {
            name: name.to_owned(),
            message: format!("refused it ({status}): {why}{hint}"),
        });
    }
    Ok(v)
}

/// Any server speaking OpenAI's `/chat/completions`.
pub struct OpenAi {
    base: String,
    key: Option<String>,
    model: String,
    agent: ureq::Agent,
}

impl OpenAi {
    /// `base`: up to `/v1`, like `https://api.openai.com/v1` or `http://saturn:11434/v1`.
    pub fn new(base: &str, key: Option<String>, model: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            key,
            model: model.to_owned(),
            agent: agent(),
        }
    }
}

impl Llm for OpenAi {
    fn ask(&self, system: &str, parts: &[Part]) -> Result<String> {
        let content: Vec<Value> = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => json!({"type": "text", "text": t}),
                Part::Png(png) => json!({"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64::encode(png))}}),
            })
            .collect();
        let body = json!({
            "model": self.model,
            "temperature": 0.4,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": content}],
        });
        let mut req = self.agent.post(format!("{}/chat/completions", self.base));
        if let Some(key) = &self.key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let v = post(req, body, &self.name())?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Llm {
                name: self.name(),
                message: "answered without any text".into(),
            })
    }

    fn name(&self) -> String {
        format!("{} at {}", self.model, self.base)
    }
}

/// Anthropic's Messages API.
pub struct Anthropic {
    base: String,
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl Anthropic {
    pub const DEFAULT_MODEL: &'static str = "claude-sonnet-5-5";

    pub fn new(key: String, model: &str) -> Self {
        Self::with_base("https://api.anthropic.com", key, model)
    }

    /// For tests: a local stand-in for the API.
    pub fn with_base(base: &str, key: String, model: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            key,
            model: model.to_owned(),
            agent: agent(),
        }
    }
}

impl Llm for Anthropic {
    fn ask(&self, system: &str, parts: &[Part]) -> Result<String> {
        let content: Vec<Value> = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => json!({"type": "text", "text": t}),
                Part::Png(png) => json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64::encode(png)}}),
            })
            .collect();
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{"role": "user", "content": content}],
        });
        let req = self
            .agent
            .post(format!("{}/v1/messages", self.base))
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01");
        let v = post(req, body, &self.name())?;
        let text: String = v["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        if text.is_empty() {
            return Err(Error::Llm {
                name: self.name(),
                message: "answered without any text".into(),
            });
        }
        Ok(text)
    }

    fn name(&self) -> String {
        format!("anthropic {}", self.model)
    }
}
