//! The pipeline: words, moments, check, render.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::llm::{Llm, Part};
use crate::media::Media;
use crate::moment::{self, Moment, Segment};
use crate::prompt;
use crate::render::Layout;
use crate::transcribe::Transcriber;
use crate::words::{self, Word};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Options {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub language: String,
    pub guidelines: String,
    pub review: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            count: 6,
            min: 15.0,
            max: 40.0,
            language: "fr".into(),
            guidelines: String::new(),
            review: true,
        }
    }
}

/// Whether a moment is worth making, and why not.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub keep: bool,
    pub reason: String,
    /// The share of its frames too dark to see (0 to 1).
    pub dark: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Made {
    pub file: PathBuf,
    pub seconds: f64,
    pub moment: Moment,
}

/// Frames darker than this (average brightness, 0 to 255) show next to nothing.
const DARK: f64 = 38.0;
/// A transcript longer than this is shown to the LLM in parts.
const WINDOW_CHARS: usize = 45_000;

pub struct Studio {
    pub media: Media,
    llm: Option<Box<dyn Llm>>,
    transcriber: Option<Box<dyn Transcriber>>,
    layout: Layout,
}

impl Studio {
    /// `work`: a folder for temporary files.
    pub fn new(
        work: PathBuf,
        llm: Option<Box<dyn Llm>>,
        transcriber: Option<Box<dyn Transcriber>>,
    ) -> Self {
        Self {
            media: Media::new(work),
            llm,
            transcriber,
            layout: Layout::default(),
        }
    }

    pub fn llm_name(&self) -> Option<String> {
        self.llm.as_ref().map(|l| l.name())
    }

    /// The video's words: its `.words.json` when there is one, else transcribed and saved.
    pub fn words(&self, video: &Path, language: &str, say: &dyn Fn(&str)) -> Result<Vec<Word>> {
        let path = words::path_for(video);
        if path.is_file() {
            return words::load(&path);
        }
        let t = self.transcriber.as_ref().ok_or_else(|| {
            Error::Setup(format!(
                "{} has no transcript yet and no transcriber is set up: set studio.transcriber (elevenlabs or whisper) or put a {} next to it",
                video.display(),
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
        })?;
        say(&format!("transcribing with {}", t.name()));
        let audio = self.media.audio(video)?;
        let words = t.words(&audio, language)?;
        let _ = fs::remove_file(&audio);
        if words.is_empty() {
            return Err(Error::Transcribe {
                name: t.name(),
                message: "found no words: is there speech in this video?".into(),
            });
        }
        words::save(&path, &words)?;
        say(&format!(
            "{} words, saved to {}",
            words.len(),
            path.display()
        ));
        Ok(words)
    }

    /// A title for the LLM: the one fetched with the video (`<name>.meta.json`), else the
    /// file's name.
    pub fn title(video: &Path) -> String {
        let meta = video.with_extension("meta.json");
        fs::read_to_string(meta)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["title"].as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                video
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
    }

    /// The LLM's moments, checked and best first: up to twice `count`, since some will not
    /// survive the frame check.
    pub fn moments(
        &self,
        video: &Path,
        words: &[Word],
        opts: &Options,
        say: &dyn Fn(&str),
    ) -> Result<Vec<Moment>> {
        let llm = self.llm.as_ref().ok_or_else(|| {
            Error::Setup("no LLM set up: set studio.llm in hive's config, or let an MCP client pick the moments (hive_transcript, hive_frames, hive_make_clip)".into())
        })?;
        let duration = self.media.duration(video)?;
        let title = Self::title(video);
        let system = prompt::pick_system(&opts.guidelines);
        let windows = windows(&words::transcript(words), WINDOW_CHARS);
        let mut found = Vec::new();
        for (i, text) in windows.iter().enumerate() {
            if windows.len() > 1 {
                say(&format!(
                    "asking {} about part {} of {}",
                    llm.name(),
                    i + 1,
                    windows.len()
                ));
            } else {
                say(&format!("asking {} for the best moments", llm.name()));
            }
            let want = (opts.count * 2).div_ceil(windows.len()) + 1;
            let answer = llm.ask(
                &system,
                &[Part::Text(prompt::pick_user(
                    &title, text, want, opts.min, opts.max,
                ))],
            )?;
            let moments = moment::parse(&answer).map_err(|e| Error::Llm {
                name: llm.name(),
                message: format!("answered without usable moments ({e})"),
            })?;
            for m in moments {
                match m.validated(duration, opts.min, opts.max) {
                    Ok(m) => found.push(m),
                    Err(why) => say(&format!("left out a moment: {why}")),
                }
            }
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut out: Vec<Moment> = Vec::new();
        for m in found {
            if !out.iter().any(|o| o.overlaps(&m)) {
                out.push(m);
            }
        }
        out.truncate(opts.count * 2);
        Ok(out)
    }

    /// Times in the source across a moment: its start, its end, and evenly in between.
    pub fn frame_times(m: &Moment, count: usize) -> Vec<f64> {
        let len = m.length();
        let count = count.max(2);
        (0..count)
            .map(|i| {
                let f = 0.03 + 0.94 * i as f64 / (count - 1) as f64;
                source_time(&m.segments, f * len)
            })
            .collect()
    }

    /// Frames across a moment as one PNG.
    pub fn frames(&self, video: &Path, m: &Moment, count: usize, out: &Path) -> Result<()> {
        self.media.sheet(video, &Self::frame_times(m, count), out)
    }

    /// Too dark to see is left out without asking; then, if the LLM can look, it judges the
    /// frames (menus, maps, nothing happening).
    pub fn check(&self, video: &Path, m: &Moment, opts: &Options) -> Result<Verdict> {
        let times = Self::frame_times(m, 6);
        let mut dark = 0;
        for t in &times {
            if self.media.brightness(video, *t)? < DARK {
                dark += 1;
            }
        }
        let dark = f64::from(dark) / times.len() as f64;
        if dark >= 0.5 {
            return Ok(Verdict {
                keep: false,
                reason: format!(
                    "too dark: {:.0} % of its frames show next to nothing",
                    dark * 100.0
                ),
                dark,
            });
        }
        let Some(llm) = self.llm.as_ref().filter(|_| opts.review) else {
            return Ok(Verdict {
                keep: true,
                reason: String::new(),
                dark,
            });
        };
        let sheet = self.media.work_file("review.png")?;
        self.media.sheet(video, &times, &sheet)?;
        let png = fs::read(&sheet).map_err(|e| Error::read(&sheet, e))?;
        let answer = llm.ask(
            prompt::REVIEW_SYSTEM,
            &[Part::Png(png), Part::Text(prompt::review_user(m))],
        )?;
        let (keep, reason) = parse_verdict(&answer);
        Ok(Verdict { keep, reason, dark })
    }

    /// Cut the clip.
    pub fn make(&self, video: &Path, words: &[Word], m: &Moment, out: &Path) -> Result<Made> {
        let segments = words::tighten(&m.segments, words, 0.15);
        if segments.is_empty() {
            return Err(Error::Input(
                "nothing left once the silences are cut".into(),
            ));
        }
        let mut words = words.to_vec();
        words::fix_numbers(&mut words);
        if let Some(dir) = out.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::write(dir, e))?;
        }
        self.media
            .render(video, &segments, &words, &m.hook, &self.layout, out)?;
        let seconds = self.media.duration(out)?;
        Ok(Made {
            file: out.to_path_buf(),
            seconds,
            moment: Moment {
                segments,
                ..m.clone()
            },
        })
    }

    /// The whole pipeline: the clips land in `out_dir` with their `captions.md` (`## <file>`
    /// sections, what clipkit and Clipdeck read) and `moments.json`.
    pub fn auto(
        &self,
        video: &Path,
        out_dir: &Path,
        opts: &Options,
        say: &dyn Fn(&str),
    ) -> Result<Vec<Made>> {
        let words = self.words(video, &opts.language, say)?;
        let moments = self.moments(video, &words, opts, say)?;
        if moments.is_empty() {
            return Err(Error::Llm {
                name: self.llm_name().unwrap_or_default(),
                message: "found no moment worth a clip".into(),
            });
        }
        say(&format!("{} moments proposed", moments.len()));
        let mut made = Vec::new();
        for m in moments {
            if made.len() >= opts.count {
                break;
            }
            let verdict = self.check(video, &m, opts)?;
            if !verdict.keep {
                say(&format!("left out \"{}\": {}", m.hook, verdict.reason));
                continue;
            }
            let file = out_dir.join(format!("{:02}-{}.mp4", made.len() + 1, slug(&m.hook)));
            say(&format!("making \"{}\" ({:.0} s)", m.hook, m.length()));
            let clip = self.make(video, &words, &m, &file)?;
            write_caption(out_dir, &clip, &Self::title(video))?;
            made.push(clip);
        }
        let manifest = out_dir.join("moments.json");
        let text = serde_json::to_string_pretty(&made).map_err(|e| Error::write(&manifest, e))?;
        fs::write(&manifest, text).map_err(|e| Error::write(&manifest, e))?;
        Ok(made)
    }
}

/// Where in the source a time of the clip falls (parts back to back).
fn source_time(segments: &[Segment], mut t: f64) -> f64 {
    for s in segments {
        if t <= s.len() {
            return s.start + t;
        }
        t -= s.len();
    }
    segments.last().map_or(0.0, |s| s.end - 0.05)
}

/// A transcript cut into parts of at most `max` characters, on line ends.
fn windows(transcript: &str, max: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in transcript.lines() {
        let cur = out.last_mut().expect("never empty");
        if !cur.is_empty() && cur.len() + line.len() > max {
            out.push(String::new());
        }
        let cur = out.last_mut().expect("never empty");
        cur.push_str(line);
        cur.push('\n');
    }
    out
}

fn parse_verdict(answer: &str) -> (bool, String) {
    let json = answer
        .find('{')
        .and_then(|s| answer[s..].rfind('}').map(|e| &answer[s..=s + e]))
        .and_then(|t| serde_json::from_str::<Value>(t).ok());
    match json {
        Some(v) => (
            v["keep"].as_bool().unwrap_or(true),
            v["reason"].as_str().unwrap_or("").to_owned(),
        ),
        // unreadable: make it, a person still looks before posting
        None => (true, String::new()),
    }
}

/// A file name from a hook: "Quand t'as 6 étoiles" → "quand-t-as-6-etoiles".
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(fold) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "clip".into() } else { out }
}

fn fold(c: char) -> Vec<char> {
    let base = match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
        'ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' => 'i',
        'ô' | 'ö' | 'ó' | 'õ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ÿ' => 'y',
        'À' | 'Â' | 'Ä' => 'A',
        'Ç' => 'C',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Î' | 'Ï' => 'I',
        'Ô' | 'Ö' => 'O',
        'Ù' | 'Û' | 'Ü' => 'U',
        'œ' => return vec!['o', 'e'],
        'æ' => return vec!['a', 'e'],
        other => other,
    };
    vec![base]
}

/// Add the clip's section to `captions.md` (replacing an older one for the same file).
pub fn write_caption(dir: &Path, clip: &Made, title: &str) -> Result<()> {
    let path = dir.join("captions.md");
    let name = clip.file.file_name().unwrap_or_default().to_string_lossy();
    let mut text =
        fs::read_to_string(&path).unwrap_or_else(|_| format!("# Clips from \"{title}\"\n"));
    let header = format!("## {name}\n");
    if let Some(start) = text.find(&header) {
        let rest = &text[start + header.len()..];
        let end = rest
            .find("\n## ")
            .map_or(text.len(), |e| start + header.len() + e + 1);
        text.replace_range(start..end, "");
    }
    if !text.ends_with("\n\n") {
        text.push('\n');
    }
    text.push_str(&format!("{header}{}\n", clip.moment.caption.trim()));
    fs::write(&path, text).map_err(|e| Error::write(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(segs: &[(f64, f64)]) -> Moment {
        Moment {
            segments: segs
                .iter()
                .map(|&(start, end)| Segment { start, end })
                .collect(),
            hook: "h".into(),
            caption: "c".into(),
            kind: String::new(),
            why: String::new(),
            score: 0.0,
        }
    }

    #[test]
    fn frame_times_follow_the_parts() {
        let m = moment(&[(100.0, 110.0), (200.0, 210.0)]);
        let t = Studio::frame_times(&m, 6);
        assert_eq!(t.len(), 6);
        assert!((t[0] - 100.6).abs() < 1e-9);
        assert!(t[5] > 209.0 && t[5] < 210.0);
        assert!(
            t.iter()
                .all(|x| (100.0..=110.0).contains(x) || (200.0..=210.0).contains(x))
        );
    }

    #[test]
    fn long_transcripts_are_cut_on_line_ends() {
        let text = "00:00.0 aaaa\n00:01.0 bbbb\n00:02.0 cccc\n";
        let w = windows(text, 28);
        assert_eq!(w, vec!["00:00.0 aaaa\n00:01.0 bbbb\n", "00:02.0 cccc\n"]);
    }

    #[test]
    fn verdicts_default_to_keep_when_unreadable() {
        assert_eq!(
            parse_verdict(r#"{"keep": false, "reason": "a menu"}"#),
            (false, "a menu".into())
        );
        assert_eq!(parse_verdict("looks fine"), (true, String::new()));
    }

    #[test]
    fn slugs_are_plain_and_short() {
        assert_eq!(
            slug("Quand t'as 6 étoiles dans GTA 6 ?"),
            "quand-t-as-6-etoiles-dans-gta-6"
        );
        assert_eq!(slug("💀"), "clip");
    }

    #[test]
    fn captions_md_keeps_one_section_per_clip() {
        let dir = std::env::temp_dir().join(format!("hive-studio-cap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut clip = Made {
            file: dir.join("01-a.mp4"),
            seconds: 20.0,
            moment: moment(&[(0.0, 20.0)]),
        };
        clip.moment.caption = "first #tag".into();
        write_caption(&dir, &clip, "Video").unwrap();
        clip.moment.caption = "second #tag".into();
        write_caption(&dir, &clip, "Video").unwrap();
        let text = fs::read_to_string(dir.join("captions.md")).unwrap();
        assert_eq!(text, "# Clips from \"Video\"\n\n## 01-a.mp4\nsecond #tag\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
