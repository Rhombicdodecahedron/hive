//! A word-timed transcript: what is said, and when, to the hundredth of a second.
//!
//! Saved next to the video as `<name>.words.json`, `[{"w": "Salut", "s": 0.12, "e": 0.4}, …]`
//! (the format clipkit writes too, so either can read the other's).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::moment::Segment;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub w: String,
    pub s: f64,
    pub e: f64,
}

/// A line of the transcript as the LLM reads it: where it starts, and what is said.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start: f64,
    pub text: String,
}

/// Words that open a sentence without saying anything: a clip never starts on one.
const FILLERS: &[&str] = &[
    "euh", "heu", "hum", "hm", "hmm", "bon", "alors", "donc", "ok", "okay", "voilà", "hein", "ben",
    "bah", "ouais", "et", "so", "uh", "um", "well", "like", "and",
];

pub fn path_for(video: &Path) -> PathBuf {
    video.with_extension("words.json")
}

pub fn load(path: &Path) -> Result<Vec<Word>> {
    let text = fs::read_to_string(path).map_err(|e| Error::read(path, e))?;
    serde_json::from_str(&text).map_err(|e| Error::read(path, e))
}

pub fn save(path: &Path, words: &[Word]) -> Result<()> {
    let text = serde_json::to_string(words).map_err(|e| Error::write(path, e))?;
    fs::write(path, text).map_err(|e| Error::write(path, e))
}

/// Group words into short lines: a new line after a pause, a sentence's end or 28 words.
pub fn lines(words: &[Word]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut cur: Vec<&Word> = Vec::new();
    for w in words {
        if let Some(last) = cur.last()
            && (w.s - last.e > 0.9 || cur.len() >= 28)
        {
            out.push(join(&cur));
            cur.clear();
        }
        cur.push(w);
        if cur.len() > 8 && w.w.ends_with(['.', '?', '!']) {
            out.push(join(&cur));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(join(&cur));
    }
    out
}

fn join(words: &[&Word]) -> Line {
    Line {
        start: words[0].s,
        text: words
            .iter()
            .map(|w| w.w.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// The transcript as text, one timed line per line: `12:03.4 what is said`.
pub fn transcript(words: &[Word]) -> String {
    lines(words)
        .iter()
        .map(|l| format!("{} {}\n", clock(l.start), l.text))
        .collect()
}

/// `723.4` → `12:03.4`; past an hour, `1:02:03.0`.
pub fn clock(t: f64) -> String {
    let t = t.max(0.0);
    let tenths = (t * 10.0).round() as u64;
    let (h, m, s, d) = (
        tenths / 36000,
        tenths / 600 % 60,
        tenths / 10 % 60,
        tenths % 10,
    );
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{d}")
    } else {
        format!("{m:02}:{s:02}.{d}")
    }
}

/// `12:03.4`, `1:02:03`, `723.4` → seconds.
pub fn parse_clock(text: &str) -> Option<f64> {
    let parts: Vec<f64> = text
        .trim()
        .split(':')
        .map(|p| p.trim().replace(',', ".").parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| *p < 0.0) {
        return None;
    }
    Some(parts.iter().fold(0.0, |acc, p| acc * 60.0 + p))
}

/// Start every part on its first real word: silence and fillers ("euh", "alors"…) go, since
/// viewers decide in the first second. Ends stay: the payoff often comes after the voice.
pub fn tighten(segments: &[Segment], words: &[Word], lead: f64) -> Vec<Segment> {
    segments
        .iter()
        .filter_map(|seg| {
            let mut start = seg.start;
            let first = words
                .iter()
                .filter(|w| w.s >= seg.start - 0.05 && w.s < seg.end)
                .find(|w| {
                    let core =
                        w.w.trim_matches(|c: char| !c.is_alphanumeric())
                            .to_lowercase();
                    !core.is_empty() && !FILLERS.contains(&core.as_str())
                });
            if let Some(w) = first {
                start = start.max(w.s - lead);
            }
            (seg.end - start > 0.5).then_some(Segment {
                start,
                end: seg.end,
            })
        })
        .collect()
}

/// Spoken numbers after a name in capitals become digits: "GTA cinq" → "GTA 5".
pub fn fix_numbers(words: &mut [Word]) {
    const NUMBERS: &[(&str, &str)] = &[
        ("un", "1"),
        ("deux", "2"),
        ("trois", "3"),
        ("quatre", "4"),
        ("cinq", "5"),
        ("six", "6"),
        ("sept", "7"),
        ("v", "5"),
        ("vi", "6"),
        ("five", "5"),
        ("six", "6"),
    ];
    for i in 1..words.len() {
        let prev = &words[i - 1].w;
        let is_name = prev.chars().count() >= 2 && prev.chars().all(|c| c.is_ascii_uppercase());
        if !is_name {
            continue;
        }
        let word = &words[i].w;
        let core_len = word.trim_end_matches(['.', ',', '!', '?']).len();
        let (core, rest) = word.split_at(core_len);
        if let Some((_, digit)) = NUMBERS.iter().find(|(n, _)| core.eq_ignore_ascii_case(n)) {
            words[i].w = format!("{digit}{rest}");
        }
    }
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
    fn clock_goes_both_ways() {
        assert_eq!(clock(723.44), "12:03.4");
        assert_eq!(clock(3723.0), "1:02:03.0");
        assert_eq!(parse_clock("12:03.4"), Some(723.4));
        assert_eq!(parse_clock("1:02:03"), Some(3723.0));
        assert_eq!(parse_clock("75,5"), Some(75.5));
        assert_eq!(parse_clock("a:b"), None);
        assert_eq!(parse_clock("-3"), None);
    }

    #[test]
    fn lines_break_on_pauses_and_sentences() {
        let words = vec![
            w("Salut", 0.0, 0.3),
            w("à", 0.3, 0.4),
            w("tous.", 0.4, 0.8),
            w("Ensuite", 2.5, 2.9),
        ];
        let l = lines(&words);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "Salut à tous.");
        assert_eq!(l[1].start, 2.5);
        assert_eq!(
            transcript(&words),
            "00:00.0 Salut à tous.\n00:02.5 Ensuite\n"
        );
    }

    #[test]
    fn tighten_skips_silence_and_fillers_but_keeps_the_end() {
        let words = vec![
            w("Euh,", 10.4, 10.6),
            w("alors", 10.7, 10.9),
            w("regardez", 11.2, 11.6),
        ];
        let out = tighten(
            &[Segment {
                start: 10.0,
                end: 20.0,
            }],
            &words,
            0.15,
        );
        assert_eq!(out.len(), 1);
        assert!((out[0].start - 11.05).abs() < 1e-9);
        assert_eq!(out[0].end, 20.0);
    }

    #[test]
    fn numbers_after_a_name_become_digits() {
        let mut words = vec![
            w("GTA", 0.0, 0.2),
            w("cinq,", 0.2, 0.4),
            w("cinq", 0.5, 0.6),
        ];
        fix_numbers(&mut words);
        assert_eq!(words[1].w, "5,");
        assert_eq!(words[2].w, "cinq");
    }
}
