//! How a clip looks, as ffmpeg filters: the gameplay at full width in the middle of a 9:16
//! frame, a blurred copy of it behind, the hook in a white box above, and the words below,
//! one to three at a time, the one being said in colour.

use std::path::Path;

use crate::media::FONT_FAMILY;
use crate::moment::Segment;
use crate::words::Word;

#[derive(Debug, Clone)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub hook_size: u32,
    pub hook_line: u32,
    /// Characters per hook line before it wraps.
    pub hook_wrap: usize,
    pub caption_size: u32,
    /// Words on screen at once.
    pub caption_words: usize,
    /// The word being said, in ASS colour (&HBBGGRR&): yellow.
    pub highlight: String,
    pub uppercase: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            width: 1080,
            height: 1920,
            hook_size: 62,
            hook_line: 96,
            hook_wrap: 24,
            caption_size: 76,
            caption_words: 3,
            highlight: "&H0AD6FF&".into(),
            uppercase: true,
        }
    }
}

impl Layout {
    /// Height of the gameplay band (16:9 at full width).
    fn band(&self) -> u32 {
        (self.width * 9 / 16) / 2 * 2
    }

    fn band_top(&self) -> u32 {
        (self.height - self.band()) / 2
    }

    /// Captions sit under the gameplay.
    fn caption_margin(&self) -> u32 {
        self.height - (self.band_top() + self.band()) - 260
    }
}

/// For a path inside an ffmpeg filter argument.
pub fn escape_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// The hook cut into lines that fit, never leaving a number or a French "?" alone on one.
pub fn hook_lines(hook: &str, layout: &Layout) -> Vec<String> {
    let mut text = hook.trim().to_owned();
    for (from, to) in [
        (" ?", "\u{a0}?"),
        (" !", "\u{a0}!"),
        (" :", "\u{a0}:"),
        (" ;", "\u{a0};"),
    ] {
        text = text.replace(from, to);
    }
    // "GTA 6": the number stays with its name
    let words: Vec<&str> = text.split(' ').collect();
    let mut glued: Vec<String> = Vec::new();
    for w in words {
        let head = w.split('\u{a0}').next().unwrap_or(w);
        let short_number =
            !head.is_empty() && head.len() <= 2 && head.chars().all(|c| c.is_ascii_digit());
        match glued.last_mut() {
            Some(prev) if short_number => {
                prev.push('\u{a0}');
                prev.push_str(w);
            }
            _ => glued.push(w.to_owned()),
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for w in glued {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + w.chars().count() <= layout.hook_wrap => {
                line.push(' ');
                line.push_str(&w);
            }
            _ => lines.push(w),
        }
    }
    lines.retain(|l| !l.is_empty());
    lines
}

fn ass_time(t: f64) -> String {
    let t = t.max(0.0);
    let cs = (t * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}

/// One ASS event per word: the line grows as each word is said, the current one in colour,
/// and goes as soon as the voice stops. Times are in the clip (parts back to back).
pub fn captions(words: &[Word], segments: &[Segment], layout: &Layout) -> Vec<String> {
    let mut events = Vec::new();
    let mut offset = 0.0;
    for seg in segments {
        let ws: Vec<&Word> = words
            .iter()
            .filter(|w| w.s >= seg.start - 0.05 && w.e <= seg.end + 0.05)
            .collect();
        let mut chunks: Vec<Vec<&Word>> = Vec::new();
        let mut cur: Vec<&Word> = Vec::new();
        for w in ws {
            if let (Some(first), Some(last)) = (cur.first(), cur.last())
                && (cur.len() >= layout.caption_words || w.s - last.e > 0.35 || w.e - first.s > 1.3)
            {
                chunks.push(std::mem::take(&mut cur));
            }
            cur.push(w);
            if w.w.ends_with(['.', '!', '?', ',']) {
                chunks.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            chunks.push(cur);
        }
        for (k, chunk) in chunks.iter().enumerate() {
            let next = chunks.get(k + 1).map_or(seg.end, |c| c[0].s);
            let stop = (chunk[chunk.len() - 1].e + 0.12).min(next);
            for (i, w) in chunk.iter().enumerate() {
                let t0 = w.s.max(seg.start);
                let t1 = chunk.get(i + 1).map_or(stop, |n| n.s).min(seg.end);
                if t1 <= t0 {
                    continue;
                }
                let mut said: Vec<String> = chunk[..=i]
                    .iter()
                    .map(|x| {
                        let t = ass_escape(&x.w);
                        if layout.uppercase {
                            t.to_uppercase()
                        } else {
                            t
                        }
                    })
                    .collect();
                if let Some(last) = said.last_mut() {
                    *last = format!("{{\\c{}}}{last}", layout.highlight);
                }
                events.push(format!(
                    "Dialogue: 0,{},{},Cap,,0,0,0,,{}",
                    ass_time(t0 - seg.start + offset),
                    ass_time(t1 - seg.start + offset),
                    said.join(" ")
                ));
            }
        }
        offset += seg.len();
    }
    events
}

fn ass_escape(text: &str) -> String {
    text.replace(['{', '}'], "").replace('\\', "")
}

pub fn ass_file(events: &[String], layout: &Layout) -> String {
    format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\n\n[V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Cap,{FONT_FAMILY},{size},&H00FFFFFF,&H00FFFFFF,&H00000000,&H64000000,0,0,0,0,100,100,1,0,1,6,2,2,60,60,{margin},1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n{events}\n",
        w = layout.width,
        h = layout.height,
        size = layout.caption_size,
        margin = layout.caption_margin(),
        events = events.join("\n"),
    )
}

/// The whole filter graph: one input per part, joined with tiny audio fades so cuts don't
/// click, then the 9:16 frame, the hook, the captions, and loudness at -14 LUFS.
pub fn filtergraph(
    segments: &[Segment],
    hook_files: &[impl AsRef<Path>],
    subtitles: Option<&Path>,
    font: &Path,
    layout: &Layout,
) -> String {
    let (w, h) = (layout.width, layout.height);
    let (band, top) = (layout.band(), layout.band_top());
    let mut g = String::new();
    for (k, s) in segments.iter().enumerate() {
        g.push_str(&format!(
            "[{k}:v]setpts=PTS-STARTPTS,fps=30[v{k}];[{k}:a]aresample=48000,asetpts=PTS-STARTPTS,afade=t=in:d=0.05,afade=t=out:st={:.2}:d=0.08[a{k}];",
            (s.len() - 0.08).max(0.0)
        ));
    }
    for k in 0..segments.len() {
        g.push_str(&format!("[v{k}][a{k}]"));
    }
    g.push_str(&format!(
        "concat=n={}:v=1:a=1[vc][ac];[vc]split=2[bgsrc][fgsrc];\
         [bgsrc]scale={q}:{qh}:force_original_aspect_ratio=increase,crop={q}:{qh},boxblur=8:2,eq=brightness=-0.12,scale={w}:{h}[bg];\
         [fgsrc]scale={w}:{band}:force_original_aspect_ratio=decrease,pad={w}:{band}:(ow-iw)/2:(oh-ih)/2[fg];\
         [bg][fg]overlay=0:{top}",
        segments.len(),
        q = w / 4,
        qh = h / 4,
    ));
    let fonts_dir = escape_path(font.parent().unwrap_or(Path::new(".")));
    let font = escape_path(font);
    let lines = hook_files.len() as u32;
    let hook_top = top.saturating_sub(70 + lines * layout.hook_line);
    for (i, f) in hook_files.iter().enumerate() {
        g.push_str(&format!(
            ",drawtext=fontfile='{font}':textfile='{}':fontsize={}:fontcolor=black:box=1:boxcolor=white:boxborderw=18:x=(w-text_w)/2:y={}",
            escape_path(f.as_ref()),
            layout.hook_size,
            hook_top + i as u32 * layout.hook_line
        ));
    }
    if let Some(ass) = subtitles {
        g.push_str(&format!(
            ",subtitles='{}':fontsdir='{fonts_dir}'",
            escape_path(ass)
        ));
    }
    g.push_str(",fps=30,format=yuv420p[v];[ac]loudnorm=I=-14:TP=-1.5:LRA=11,aresample=48000,asetpts=N/SR/TB[aout]");
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, s: f64, e: f64) -> Word {
        Word {
            w: text.into(),
            s,
            e,
        }
    }

    #[test]
    fn hook_wraps_and_keeps_numbers_and_question_marks_attached() {
        let lines = hook_lines(
            "Moto ou vélo depuis le plus haut point de GTA 6 ?",
            &Layout::default(),
        );
        assert_eq!(
            lines,
            vec![
                "Moto ou vélo depuis le",
                "plus haut point de",
                "GTA\u{a0}6\u{a0}?"
            ]
        );
    }

    #[test]
    fn captions_build_up_word_by_word_across_parts() {
        let words = vec![
            w("Salut", 10.0, 10.3),
            w("tous.", 10.35, 10.7),
            w("Boum", 50.0, 50.4),
        ];
        let segs = [
            Segment {
                start: 9.9,
                end: 11.0,
            },
            Segment {
                start: 49.9,
                end: 51.0,
            },
        ];
        let ev = captions(&words, &segs, &Layout::default());
        assert_eq!(ev.len(), 3);
        assert!(
            ev[0].starts_with("Dialogue: 0,0:00:00.10,0:00:00.45,Cap,,0,0,0,,{\\c&H0AD6FF&}SALUT")
        );
        assert!(ev[1].ends_with("SALUT {\\c&H0AD6FF&}TOUS."));
        // the second part starts after the first one's 1.1 s
        assert!(ev[2].starts_with("Dialogue: 0,0:00:01.20,"));
    }

    #[test]
    fn graph_has_one_input_per_part_and_the_hook_lines() {
        let segs = [
            Segment {
                start: 0.0,
                end: 5.0,
            },
            Segment {
                start: 9.0,
                end: 12.0,
            },
        ];
        let g = filtergraph(
            &segs,
            &["/tmp/h0.txt", "/tmp/h1.txt"],
            Some(Path::new("/tmp/c.ass")),
            Path::new("/tmp/f.ttf"),
            &Layout::default(),
        );
        assert!(g.contains("[0:v]") && g.contains("[1:a]") && g.contains("concat=n=2"));
        assert_eq!(g.matches("drawtext").count(), 2);
        assert!(g.contains("subtitles='/tmp/c.ass':fontsdir='/tmp'"));
        assert!(g.ends_with("[aout]"));
    }
}
