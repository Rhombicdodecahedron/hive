//! Hive's studio: a long video and its transcript in, vertical clips ready to post out.
//!
//! The pipeline, each step usable on its own:
//!
//! 1. words: a word-timed transcript (`<video>.words.json`), from ElevenLabs or whisper.cpp;
//! 2. moments: an LLM reads the transcript and proposes the moments worth a clip;
//! 3. check: frames of each moment, judged dark or not by their brightness, and by the LLM
//!    when it can see pictures (menus, maps, nothing happening);
//! 4. render: ffmpeg cuts the parts, stacks them 9:16 over a blurred copy, writes the hook on
//!    top and the words underneath, as they are said.
//!
//! The LLM is a port ([`Llm`]): any OpenAI-compatible server (OpenAI, Ollama, OpenRouter…) or
//! Anthropic's API. With none configured, an MCP client (Claude) can do step 2 itself through
//! `hive mcp`, from the same transcript and pictures.

pub mod b64;
mod error;
pub mod llm;
pub mod media;
pub mod moment;
pub mod prompt;
pub mod render;
mod settings;
mod studio;
pub mod transcribe;
pub mod words;

pub use error::{Error, Result};
pub use llm::{Llm, Part};
pub use moment::{Moment, Segment};
pub use settings::Settings;
pub use studio::{Made, Options, Studio, Verdict, slug, write_caption};
pub use transcribe::Transcriber;
pub use words::Word;
