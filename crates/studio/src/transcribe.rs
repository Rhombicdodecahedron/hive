//! Word-timed transcription: ElevenLabs Scribe (fast, very good in French, paid per hour of
//! audio) or whisper.cpp on this computer (free, faster with a GPU).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::words::Word;
use crate::{Error, Result};

pub trait Transcriber: Send + Sync {
    /// `audio`: 16 kHz mono WAV, as [`crate::media::Media::audio`] makes it.
    fn words(&self, audio: &Path, language: &str) -> Result<Vec<Word>>;
    fn name(&self) -> String;
}

pub struct ElevenLabs {
    key: String,
    model: String,
    url: String,
}

impl ElevenLabs {
    pub const DEFAULT_MODEL: &'static str = "scribe_v1";

    pub fn new(key: String, model: &str) -> Self {
        Self {
            key,
            model: model.to_owned(),
            url: "https://api.elevenlabs.io/v1/speech-to-text".into(),
        }
    }
}

impl Transcriber for ElevenLabs {
    fn words(&self, audio: &Path, language: &str) -> Result<Vec<Word>> {
        let fail = |message: String| Error::Transcribe {
            name: self.name(),
            message,
        };
        let data = fs::read(audio).map_err(|e| Error::read(audio, e))?;
        let boundary = format!("hive{:x}", data.len() as u64 ^ 0x5eed_cafe);
        let mut body = Vec::with_capacity(data.len() + 1024);
        for (k, v) in [
            ("model_id", self.model.as_str()),
            ("language_code", language),
            ("timestamps_granularity", "word"),
            ("tag_audio_events", "false"),
        ] {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(&data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(1800)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut res = agent
            .post(&self.url)
            .header("xi-api-key", &self.key)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])
            .map_err(|e| fail(format!("isn't answering: {e}")))?;
        let status = res.status();
        let text = res
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_string()
            .unwrap_or_default();
        if !status.is_success() {
            let hint = if status.as_u16() == 401 {
                " (check ELEVENLABS_API_KEY)"
            } else {
                ""
            };
            return Err(fail(format!(
                "refused it ({status}): {}{hint}",
                text.chars().take(300).collect::<String>()
            )));
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| fail(format!("answered oddly: {e}")))?;
        Ok(v["words"]
            .as_array()
            .map(|ws| {
                ws.iter()
                    .filter(|w| w["type"].as_str().unwrap_or("word") == "word")
                    .filter_map(|w| {
                        let t = w["text"].as_str()?.trim();
                        (!t.is_empty()).then(|| Word {
                            w: t.to_owned(),
                            s: w["start"].as_f64().unwrap_or(0.0),
                            e: w["end"].as_f64().unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn name(&self) -> String {
        format!("ElevenLabs {}", self.model)
    }
}

/// whisper.cpp's command line (`whisper-cli`), one word per segment.
pub struct WhisperCpp {
    bin: String,
    model: PathBuf,
}

impl WhisperCpp {
    pub fn new(bin: &str, model: PathBuf) -> Self {
        Self {
            bin: bin.to_owned(),
            model,
        }
    }
}

impl Transcriber for WhisperCpp {
    fn words(&self, audio: &Path, language: &str) -> Result<Vec<Word>> {
        if !self.model.is_file() {
            return Err(Error::Setup(format!(
                "no whisper model at {}: download one (ggml-large-v3-turbo.bin for instance) and set studio.whisper_model",
                self.model.display()
            )));
        }
        let out = audio.with_extension("whisper");
        let status = Command::new(&self.bin)
            .args(["-m"])
            .arg(&self.model)
            .args(["-f"])
            .arg(audio)
            .args(["-l", language, "-ml", "1", "-sow", "-oj", "-np", "-of"])
            .arg(&out)
            .status()
            .map_err(|_| Error::Missing("whisper-cli (whisper.cpp)"))?;
        if !status.success() {
            return Err(Error::Tool {
                tool: "whisper-cli",
                message: format!("exited with {status}"),
            });
        }
        let json = out.with_extension("whisper.json");
        let text = fs::read_to_string(&json).map_err(|e| Error::read(&json, e))?;
        let _ = fs::remove_file(&json);
        parse_whisper(&text).map_err(|e| Error::read(json, e))
    }

    fn name(&self) -> String {
        format!("whisper.cpp {}", self.model.display())
    }
}

/// whisper.cpp's `-oj` output with `-ml 1`: one segment per word, times in milliseconds.
fn parse_whisper(text: &str) -> Result<Vec<Word>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let segs = v["transcription"]
        .as_array()
        .ok_or("no \"transcription\" list")?;
    Ok(segs
        .iter()
        .filter_map(|s| {
            let t = s["text"].as_str()?.trim();
            if t.is_empty() || t.starts_with('[') {
                return None;
            }
            Some(Word {
                w: t.to_owned(),
                s: s["offsets"]["from"].as_f64()? / 1000.0,
                e: s["offsets"]["to"].as_f64()? / 1000.0,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::parse_whisper;

    #[test]
    fn reads_whisper_cpp_word_segments() {
        let json = r#"{"transcription": [
            {"offsets": {"from": 0, "to": 320}, "text": " Salut"},
            {"offsets": {"from": 320, "to": 400}, "text": ""},
            {"offsets": {"from": 400, "to": 900}, "text": " [_BEG_]"},
            {"offsets": {"from": 900, "to": 1210}, "text": " tous."}]}"#;
        let w = parse_whisper(json).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].w, "Salut");
        assert_eq!(w[1].s, 0.9);
        assert_eq!(w[1].e, 1.21);
    }
}
