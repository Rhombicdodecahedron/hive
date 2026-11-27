//! The `[studio]` table of hive's config.toml. Keys never go in the file: it names the
//! environment variable that holds them.
//!
//! ```toml
//! [studio]
//! llm = "openai"                       # or "anthropic"
//! base_url = "http://saturn:11434/v1"  # Ollama; default https://api.openai.com/v1
//! model = "qwen2.5vl:7b"
//! transcriber = "elevenlabs"           # or "whisper"
//! language = "fr"
//! guidelines = "Hashtags: #gta6 #lasalle #gta6fr. Tutoie."
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::llm::{Anthropic, Llm, OpenAi};
use crate::studio::Options;
use crate::transcribe::{ElevenLabs, Transcriber, WhisperCpp};
use crate::{Error, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "openai" (any OpenAI-compatible server) or "anthropic"; none: an MCP client picks.
    pub llm: Option<String>,
    pub model: Option<String>,
    /// For "openai": the server, up to /v1.
    pub base_url: Option<String>,
    /// The environment variable holding the LLM's key (default OPENAI_API_KEY or
    /// ANTHROPIC_API_KEY).
    pub api_key_env: Option<String>,
    /// "elevenlabs" (ELEVENLABS_API_KEY) or "whisper" (whisper.cpp).
    pub transcriber: Option<String>,
    /// ElevenLabs' model (default scribe_v1).
    pub transcriber_model: Option<String>,
    /// whisper.cpp's command (default whisper-cli) and model file.
    pub whisper_bin: Option<String>,
    pub whisper_model: Option<PathBuf>,
    /// The videos' language, for the transcriber (default "fr").
    pub language: Option<String>,
    /// Clips per video (default 6), and their length in seconds (default 15 to 40).
    pub count: Option<usize>,
    pub min_seconds: Option<f64>,
    pub max_seconds: Option<f64>,
    /// The account's own rules, added to the LLM's: hashtags, tone, what worked.
    pub guidelines: Option<String>,
    /// Show the LLM frames of each moment before making it (default true).
    pub review: Option<bool>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Settings {
    pub fn llm(&self) -> Result<Option<Box<dyn Llm>>> {
        match self.llm.as_deref().map(str::trim) {
            None | Some("") | Some("none") => Ok(None),
            Some("anthropic") => {
                let var = self.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY");
                let key = env(var).ok_or_else(|| {
                    Error::Setup(format!("studio.llm is anthropic: put the API key in {var}"))
                })?;
                let model = self.model.as_deref().unwrap_or(Anthropic::DEFAULT_MODEL);
                Ok(Some(Box::new(Anthropic::new(key, model))))
            }
            Some("openai") => {
                let model = self.model.as_deref().ok_or_else(|| {
                    Error::Setup("studio.llm is openai: set studio.model (and studio.base_url for Ollama or another server)".into())
                })?;
                let base = self
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1");
                let key = env(self.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY"));
                Ok(Some(Box::new(OpenAi::new(base, key, model))))
            }
            Some(other) => Err(Error::Setup(format!(
                "studio.llm is \"{other}\": use \"openai\" or \"anthropic\""
            ))),
        }
    }

    /// None when no transcriber is set up: videos then need their `.words.json` already.
    pub fn transcriber(&self) -> Result<Option<Box<dyn Transcriber>>> {
        let which = self.transcriber.as_deref().map(str::trim).or_else(|| {
            if env("ELEVENLABS_API_KEY").is_some() {
                Some("elevenlabs")
            } else if self.whisper_model.is_some() {
                Some("whisper")
            } else {
                None
            }
        });
        match which {
            None | Some("") | Some("none") => Ok(None),
            Some("elevenlabs") => {
                let key = env("ELEVENLABS_API_KEY").ok_or_else(|| {
                    Error::Setup(
                        "studio.transcriber is elevenlabs: put the API key in ELEVENLABS_API_KEY"
                            .into(),
                    )
                })?;
                let model = self
                    .transcriber_model
                    .as_deref()
                    .unwrap_or(ElevenLabs::DEFAULT_MODEL);
                Ok(Some(Box::new(ElevenLabs::new(key, model))))
            }
            Some("whisper") => {
                let model = self.whisper_model.clone().ok_or_else(|| {
                    Error::Setup("studio.transcriber is whisper: set studio.whisper_model to a ggml model file".into())
                })?;
                let bin = self.whisper_bin.as_deref().unwrap_or("whisper-cli");
                Ok(Some(Box::new(WhisperCpp::new(bin, model))))
            }
            Some(other) => Err(Error::Setup(format!(
                "studio.transcriber is \"{other}\": use \"elevenlabs\" or \"whisper\""
            ))),
        }
    }

    pub fn options(&self) -> Options {
        let d = Options::default();
        Options {
            count: self.count.unwrap_or(d.count).max(1),
            min: self.min_seconds.unwrap_or(d.min),
            max: self.max_seconds.unwrap_or(d.max),
            language: self.language.clone().unwrap_or(d.language),
            guidelines: self.guidelines.clone().unwrap_or_default(),
            review: self.review.unwrap_or(d.review),
        }
    }
}
