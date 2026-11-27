//! ffmpeg and ffprobe: length, audio for the transcriber, frames to look at, and the clip.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::moment::Segment;
use crate::render::{self, Layout};
use crate::words::{Word, clock};
use crate::{Error, Result};

/// The font hooks and words are written in (Poppins ExtraBold, SIL Open Font License), so a
/// clip looks the same on every computer.
const FONT: &[u8] = include_bytes!("../fonts/Poppins-ExtraBold.ttf");
pub const FONT_FAMILY: &str = "Poppins ExtraBold";

pub struct Media {
    ffmpeg: String,
    ffprobe: String,
    /// Where temporary files go (audio, frames, subtitles); the font is kept there too.
    work: PathBuf,
}

impl Media {
    pub fn new(work: PathBuf) -> Self {
        Self {
            ffmpeg: std::env::var("HIVE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()),
            ffprobe: std::env::var("HIVE_FFPROBE").unwrap_or_else(|_| "ffprobe".into()),
            work,
        }
    }

    fn run(&self, tool: &'static str, cmd: &mut Command) -> Result<Output> {
        let out = cmd.output().map_err(|_| Error::Missing(tool))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(Error::Tool {
                tool,
                message: last.trim().to_owned(),
            });
        }
        Ok(out)
    }

    fn work_dir(&self) -> Result<&Path> {
        fs::create_dir_all(&self.work).map_err(|e| Error::write(&self.work, e))?;
        Ok(&self.work)
    }

    /// A path for a temporary file in the work folder.
    pub fn work_file(&self, name: &str) -> Result<PathBuf> {
        Ok(self.work_dir()?.join(name))
    }

    /// The font file, written once into the work folder.
    pub fn font(&self) -> Result<PathBuf> {
        let dir = self.work_dir()?.join("fonts");
        let path = dir.join("Poppins-ExtraBold.ttf");
        if fs::metadata(&path).map(|m| m.len()).ok() != Some(FONT.len() as u64) {
            fs::create_dir_all(&dir).map_err(|e| Error::write(&dir, e))?;
            fs::write(&path, FONT).map_err(|e| Error::write(&path, e))?;
        }
        Ok(path)
    }

    /// Seconds.
    pub fn duration(&self, video: &Path) -> Result<f64> {
        let out = self.run(
            "ffprobe",
            Command::new(&self.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "format=duration",
                    "-of",
                    "csv=p=0",
                ])
                .arg(video),
        )?;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .map_err(|_| Error::read(video, "not a video ffprobe can read"))
    }

    /// 16 kHz mono WAV, what transcribers want.
    pub fn audio(&self, video: &Path) -> Result<PathBuf> {
        let out = self.work_dir()?.join("audio.wav");
        self.run(
            "ffmpeg",
            Command::new(&self.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(video)
                .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
                .arg(&out),
        )?;
        Ok(out)
    }

    /// The average brightness (0 to 255) of the frame at `t`.
    pub fn brightness(&self, video: &Path, t: f64) -> Result<f64> {
        let out = self.run(
            "ffmpeg",
            Command::new(&self.ffmpeg)
                .args(["-hide_banner", "-ss", &format!("{t:.2}"), "-i"])
                .arg(video)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=160:-2,signalstats,metadata=print:key=lavfi.signalstats.YAVG",
                    "-f",
                    "null",
                    "-",
                ]),
        )?;
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.split("YAVG=").nth(1)?.trim().parse().ok())
            .ok_or_else(|| Error::Tool {
                tool: "ffmpeg",
                message: "no brightness reading".into(),
            })
    }

    /// Frames at `times`, side by side in a grid of 3 columns, each with its time in the
    /// corner: what a person (or an LLM) looks at to judge a moment.
    pub fn sheet(&self, video: &Path, times: &[f64], out: &Path) -> Result<()> {
        if times.is_empty() {
            return Err(Error::Input("no frame to show".into()));
        }
        let font = self.font()?;
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        for t in times {
            cmd.args(["-ss", &format!("{t:.2}"), "-i"]).arg(video);
        }
        let cols = times.len().min(3);
        let rows = times.len().div_ceil(cols);
        let mut graph = String::new();
        for (i, t) in times.iter().enumerate() {
            graph.push_str(&format!(
                "[{i}:v]trim=end_frame=1,scale=480:270:force_original_aspect_ratio=decrease,pad=480:270:(ow-iw)/2:(oh-ih)/2,setsar=1,drawtext=fontfile='{}':text='{}':x=8:y=8:fontsize=24:fontcolor=yellow:borderw=3:bordercolor=black[f{i}];",
                render::escape_path(&font),
                clock(*t).replace(':', "\\:")
            ));
        }
        let blanks = rows * cols - times.len();
        for b in 0..blanks {
            graph.push_str(&format!("color=black:s=480x270:d=0.04[b{b}];"));
        }
        let cells: String = (0..times.len())
            .map(|i| format!("[f{i}]"))
            .chain((0..blanks).map(|b| format!("[b{b}]")))
            .collect();
        let layout: Vec<String> = (0..rows * cols)
            .map(|k| format!("{}_{}", (k % cols) * 480, (k / cols) * 270))
            .collect();
        graph.push_str(&format!(
            "{cells}xstack=inputs={}:layout={}[out]",
            rows * cols,
            layout.join("|")
        ));
        self.run(
            "ffmpeg",
            cmd.args(["-filter_complex", &graph, "-map", "[out]", "-frames:v", "1"])
                .arg(out),
        )?;
        Ok(())
    }

    /// Cut, stack and caption the parts into a 1080×1920 clip.
    pub fn render(
        &self,
        video: &Path,
        segments: &[Segment],
        words: &[Word],
        hook: &str,
        layout: &Layout,
        out: &Path,
    ) -> Result<()> {
        if segments.is_empty() {
            return Err(Error::Input("no part to cut".into()));
        }
        let work = self.work_dir()?;
        let font = self.font()?;
        let mut hook_files = Vec::new();
        for (i, line) in render::hook_lines(hook, layout).iter().enumerate() {
            let p = work.join(format!("hook{i}.txt"));
            fs::write(&p, line).map_err(|e| Error::write(&p, e))?;
            hook_files.push(p);
        }
        let ass = work.join("captions.ass");
        let events = render::captions(words, segments, layout);
        let subs = if events.is_empty() {
            None
        } else {
            fs::write(&ass, render::ass_file(&events, layout))
                .map_err(|e| Error::write(&ass, e))?;
            Some(ass.as_path())
        };
        let graph = render::filtergraph(segments, &hook_files, subs, &font, layout);
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        for s in segments {
            cmd.args([
                "-ss",
                &format!("{:.3}", s.start),
                "-t",
                &format!("{:.3}", s.len()),
                "-i",
            ])
            .arg(video);
        }
        cmd.args(["-filter_complex", &graph, "-map", "[v]", "-map", "[aout]"])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-profile:v",
                "high",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ar",
                "48000",
                "-movflags",
                "+faststart",
            ])
            .arg(out);
        self.run("ffmpeg", &mut cmd)?;
        Ok(())
    }
}
