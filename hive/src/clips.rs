//! `hive clips` (built with `--features studio`): a long video in, vertical clips out.
//!
//! The moments are picked by the LLM set up in `[studio]`, or, through `hive mcp`, by the MCP
//! client itself: it reads the transcript (`hive_transcript`), looks at frames
//! (`hive_frames`) and cuts what it chose (`hive_make_clip`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use hive_studio::moment::parse_segments;
use hive_studio::{Moment, Options, Settings, Studio, words};
use serde_json::{Value, json};

use crate::home::Home;

#[derive(Subcommand)]
pub enum ClipsAction {
    /// All of it: transcribe if needed, ask the LLM, look at the frames, cut the clips.
    Auto {
        video: PathBuf,
        /// Where the clips go (default: a folder next to the video).
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// How many clips (default from the config, or 6).
        #[arg(long)]
        count: Option<usize>,
        /// Don't show the LLM the frames before cutting (dark moments are still left out).
        #[arg(long)]
        no_review: bool,
    },
    /// The video's transcript, one timed line per line (transcribed first if needed).
    Transcript { video: PathBuf },
    /// Frames across parts of the video, as one picture.
    Frames {
        video: PathBuf,
        /// The parts: 12:03-12:30,13:10-13:20
        segments: String,
        #[arg(long, default_value_t = 6)]
        count: usize,
        #[arg(long, short, default_value = "frames.png")]
        out: PathBuf,
    },
    /// Cut one clip from parts you chose.
    Make {
        video: PathBuf,
        /// The parts, in order: 12:03-12:30,13:10-13:20
        segments: String,
        /// The line written on top.
        #[arg(long)]
        hook: String,
        /// The post's description, added to captions.md next to the clip.
        #[arg(long, default_value = "")]
        caption: String,
        #[arg(long, short)]
        out: PathBuf,
    },
}

fn open(home: &Home) -> Result<(Studio, Options)> {
    let settings: Settings = match home.config()?.studio {
        Some(table) => table
            .try_into()
            .context("the [studio] table of the config isn't valid")?,
        None => Settings::default(),
    };
    let studio = Studio::new(home.studio_dir(), settings.llm()?, settings.transcriber()?);
    Ok((studio, settings.options()))
}

fn video_path(video: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(video).with_context(|| format!("{} not found", video.display()))
}

fn default_out(video: &Path) -> PathBuf {
    let stem = video.file_stem().unwrap_or_default().to_string_lossy();
    video.with_file_name(format!("{stem}-clips"))
}

fn moment_from(segments: &str, hook: &str, caption: &str) -> Result<Moment> {
    Ok(Moment {
        segments: parse_segments(segments)?,
        hook: hook.trim().to_owned(),
        caption: caption.trim().to_owned(),
        kind: String::new(),
        why: String::new(),
        score: 0.0,
    })
}

fn make(studio: &Studio, video: &Path, m: &Moment, out: &Path) -> Result<hive_studio::Made> {
    let words = load_words(video)?;
    let made = studio.make(video, &words, m, out)?;
    if !m.caption.is_empty() {
        let dir = out.parent().unwrap_or(Path::new("."));
        hive_studio::write_caption(dir, &made, &Studio::title(video))?;
    }
    Ok(made)
}

/// Words are optional for a hand-made clip: without them it simply has no captions.
fn load_words(video: &Path) -> Result<Vec<hive_studio::Word>> {
    let path = words::path_for(video);
    Ok(if path.is_file() {
        words::load(&path)?
    } else {
        Vec::new()
    })
}

pub fn run(action: ClipsAction) -> Result<()> {
    let home = Home::locate()?;
    let (studio, mut opts) = open(&home)?;
    let say = |line: &str| println!("  {line}");
    match action {
        ClipsAction::Auto {
            video,
            out,
            count,
            no_review,
        } => {
            let video = video_path(&video)?;
            let out = out.unwrap_or_else(|| default_out(&video));
            if let Some(c) = count {
                opts.count = c.max(1);
            }
            opts.review &= !no_review;
            let made = studio.auto(&video, &out, &opts, &say)?;
            println!("{} clips in {}", made.len(), out.display());
            for m in &made {
                println!(
                    "  {}  {:.0} s  {}",
                    m.file.file_name().unwrap_or_default().to_string_lossy(),
                    m.seconds,
                    m.moment.hook
                );
            }
        }
        ClipsAction::Transcript { video } => {
            let video = video_path(&video)?;
            let words = studio.words(&video, &opts.language, &|l| eprintln!("  {l}"))?;
            print!("{}", words::transcript(&words));
        }
        ClipsAction::Frames {
            video,
            segments,
            count,
            out,
        } => {
            let video = video_path(&video)?;
            let m = moment_from(&segments, "frames", "")?;
            studio.frames(&video, &m, count.clamp(2, 12), &out)?;
            println!("saved {}", out.display());
        }
        ClipsAction::Make {
            video,
            segments,
            hook,
            caption,
            out,
        } => {
            let video = video_path(&video)?;
            let made = make(
                &studio,
                &video,
                &moment_from(&segments, &hook, &caption)?,
                &out,
            )?;
            println!("saved {} ({:.0} s)", made.file.display(), made.seconds);
        }
    }
    Ok(())
}

// --- MCP -----------------------------------------------------------------------------------

pub fn mcp_tools() -> Value {
    let video = json!({"type": "string", "description": "Absolute path of the long video on this computer"});
    let segments = json!({"type": "string", "description": "The parts, in order, as in the transcript: \"12:03.4-12:30,13:10-13:20.5\""});
    json!([
        {"name": "hive_transcript", "description": "A long video's transcript, one line per sentence with its time (mm:ss.s). Transcribed first when it has no .words.json yet (needs a transcriber in [studio]). Read it to pick the moments worth a clip.",
         "inputSchema": {"type": "object", "properties": {"video": video}, "required": ["video"]}},
        {"name": "hive_frames", "description": "Frames across parts of the video as one picture, time in each corner: look before cutting, to leave out dark scenes, menus, maps and moments where nothing happens.",
         "inputSchema": {"type": "object", "properties": {"video": video, "segments": segments, "count": {"type": "integer", "description": "How many frames (2 to 12, default 6)"}}, "required": ["video", "segments"]}},
        {"name": "hive_make_clip", "description": "Cut a vertical 1080x1920 clip from parts of the video: gameplay in the middle over a blurred copy, the hook on top, the words underneath as they are said. 15 to 40 s works best; start on the action, keep the payoff. The caption goes into captions.md next to the clip.",
         "inputSchema": {"type": "object", "properties": {
             "video": video, "segments": segments,
             "hook": {"type": "string", "description": "The line written on top, under 60 characters, in the video's language"},
             "caption": {"type": "string", "description": "The post's description, hashtags included"},
             "out": {"type": "string", "description": "Where to save the clip (default: <video>-clips/<hook>.mp4)"}},
           "required": ["video", "segments", "hook"]}},
        {"name": "hive_make_clips", "description": "All of it with the LLM set up in hive's [studio] config: transcript, moments, frame check, clips. Takes minutes. Without an LLM set up, pick the moments yourself with hive_transcript, hive_frames and hive_make_clip.",
         "inputSchema": {"type": "object", "properties": {"video": video, "count": {"type": "integer"}, "out": {"type": "string"}}, "required": ["video"]}}
    ])
}

fn text(v: &Value) -> Value {
    json!([{"type": "text", "text": serde_json::to_string_pretty(v).unwrap_or_default()}])
}

/// The tool's content blocks, or None when the tool isn't the studio's.
pub fn mcp_call(home: &Home, name: &str, args: &Value) -> Option<Result<Value>> {
    if !matches!(
        name,
        "hive_transcript" | "hive_frames" | "hive_make_clip" | "hive_make_clips"
    ) {
        return None;
    }
    Some(mcp_run(home, name, args))
}

fn mcp_run(home: &Home, name: &str, args: &Value) -> Result<Value> {
    let (studio, mut opts) = open(home)?;
    let arg = |k: &str| args[k].as_str().map(str::to_owned);
    let video = video_path(Path::new(
        &arg("video").ok_or_else(|| anyhow!("video is required"))?,
    ))?;
    // stdout carries the protocol: progress goes to stderr
    let say = |line: &str| eprintln!("hive: {line}");
    Ok(match name {
        "hive_transcript" => {
            let words = studio.words(&video, &opts.language, &say)?;
            json!([{"type": "text", "text": words::transcript(&words)}])
        }
        "hive_frames" => {
            let m = moment_from(&arg("segments").unwrap_or_default(), "frames", "")?;
            let count = args["count"].as_u64().unwrap_or(6).clamp(2, 12) as usize;
            let out = home.studio_dir().join("frames.png");
            studio.frames(&video, &m, count, &out)?;
            let png = std::fs::read(&out)?;
            json!([
                {"type": "image", "data": hive_studio::b64::encode(&png), "mimeType": "image/png"},
                {"type": "text", "text": format!("{} frames across {}", count, m.segments_text())}
            ])
        }
        "hive_make_clip" => {
            let m = moment_from(
                &arg("segments").unwrap_or_default(),
                &arg("hook").ok_or_else(|| anyhow!("hook is required"))?,
                &arg("caption").unwrap_or_default(),
            )?;
            let out = arg("out").map(PathBuf::from).unwrap_or_else(|| {
                default_out(&video).join(format!("{}.mp4", hive_studio::slug(&m.hook)))
            });
            let made = make(&studio, &video, &m, &out)?;
            text(
                &json!({"file": made.file, "seconds": made.seconds, "segments": made.moment.segments_text()}),
            )
        }
        _ => {
            if let Some(c) = args["count"].as_u64() {
                opts.count = (c as usize).max(1);
            }
            let out = arg("out")
                .map(PathBuf::from)
                .unwrap_or_else(|| default_out(&video));
            let made = studio.auto(&video, &out, &opts, &say)?;
            text(&json!({"folder": out, "clips": made.iter().map(|m| json!({
                "file": m.file, "seconds": m.seconds, "hook": m.moment.hook,
                "caption": m.moment.caption, "segments": m.moment.segments_text()})).collect::<Vec<_>>()}))
        }
    })
}
