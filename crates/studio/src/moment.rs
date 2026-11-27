//! A moment worth a clip: which parts of the video, the hook written on top, the post's
//! description, and why it should work.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::words::{clock, parse_clock};
use crate::{Error, Result};

/// A part of the source video, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

impl Segment {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }
}

impl std::fmt::Display for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", clock(self.start), clock(self.end))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    /// One to three parts, in order; the clip plays them back to back.
    pub segments: Vec<Segment>,
    /// The line written on top of the clip.
    pub hook: String,
    /// The post's description, hashtags included.
    pub caption: String,
    /// fail, surprise, comparison, secret, funny, tip… (free text)
    #[serde(default)]
    pub kind: String,
    /// Why it should work, in a sentence.
    #[serde(default)]
    pub why: String,
    /// How sure the LLM is, 1 to 10.
    #[serde(default)]
    pub score: f64,
}

impl Moment {
    pub fn length(&self) -> f64 {
        self.segments.iter().map(Segment::len).sum()
    }

    pub fn span(&self) -> Segment {
        Segment {
            start: self.segments.first().map_or(0.0, |s| s.start),
            end: self.segments.last().map_or(0.0, |s| s.end),
        }
    }

    /// The parts as `12:03.4-12:20.0,12:31.0-12:40.5`.
    pub fn segments_text(&self) -> String {
        self.segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Put the parts in order, inside the video, without overlaps or crumbs, and check the
    /// clip lands near the wanted length.
    pub fn validated(mut self, duration: f64, min: f64, max: f64) -> Result<Self, String> {
        self.segments
            .retain(|s| s.start.is_finite() && s.end.is_finite());
        for s in &mut self.segments {
            s.start = s.start.clamp(0.0, duration);
            s.end = s.end.clamp(0.0, duration);
        }
        self.segments.retain(|s| s.len() >= 0.5);
        self.segments.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut merged: Vec<Segment> = Vec::new();
        for s in self.segments {
            match merged.last_mut() {
                Some(last) if s.start <= last.end + 0.3 => last.end = last.end.max(s.end),
                _ => merged.push(s),
            }
        }
        self.segments = merged;
        if self.segments.is_empty() {
            return Err("no usable part".into());
        }
        if self.segments.len() > 4 {
            return Err(format!("{} parts, too choppy", self.segments.len()));
        }
        let len = self.length();
        if len < min - 3.0 || len > max + 8.0 {
            return Err(format!("{len:.0} s long, wanted {min:.0} to {max:.0} s"));
        }
        self.hook = self.hook.trim().to_owned();
        self.caption = self.caption.trim().to_owned();
        if self.hook.is_empty() {
            return Err("no hook".into());
        }
        Ok(self)
    }

    /// Two moments that share most of their time are the same moment.
    pub fn overlaps(&self, other: &Moment) -> bool {
        let shared: f64 = self
            .segments
            .iter()
            .flat_map(|a| other.segments.iter().map(move |b| (a, b)))
            .map(|(a, b)| (a.end.min(b.end) - a.start.max(b.start)).max(0.0))
            .sum();
        shared > 0.4 * self.length().min(other.length())
    }
}

/// Parts written by a person: `12:03-12:30,13:10.5-13:20`.
pub fn parse_segments(text: &str) -> Result<Vec<Segment>> {
    let segs: Option<Vec<Segment>> = text
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| segment(&Value::String(p.trim().to_owned())))
        .collect();
    match segs {
        Some(s) if !s.is_empty() && s.iter().all(|s| s.end > s.start) => Ok(s),
        _ => Err(Error::Input(format!(
            "\"{text}\" isn't a list of parts: write them like 12:03-12:30,13:10.5-13:20"
        ))),
    }
}

/// Read the moments out of an LLM's answer: a JSON list, or `{"moments": [...]}`, possibly
/// inside a ```json fence or after some talk. Times may be seconds or "mm:ss.s"; a part may be
/// `{"start", "end"}` or `[start, end]`.
pub fn parse(answer: &str) -> Result<Vec<Moment>> {
    let value =
        extract_json(answer).ok_or_else(|| Error::Input("the answer has no JSON in it".into()))?;
    let list = match &value {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .get("moments")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| Error::Input("the JSON has no \"moments\" list".into()))?,
        _ => return Err(Error::Input("the JSON isn't a list of moments".into())),
    };
    Ok(list.iter().filter_map(moment).collect())
}

fn moment(v: &Value) -> Option<Moment> {
    let segments = match v.get("segments").or_else(|| v.get("parts")) {
        Some(Value::Array(parts)) => parts.iter().filter_map(segment).collect(),
        _ => vec![segment(v)?],
    };
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Some(Moment {
        segments,
        hook: text("hook"),
        caption: text("caption"),
        kind: text("kind"),
        why: text("why"),
        score: v.get("score").and_then(time).unwrap_or(0.0),
    })
}

fn segment(v: &Value) -> Option<Segment> {
    let (start, end) = match v {
        Value::Array(pair) if pair.len() == 2 => (time(&pair[0])?, time(&pair[1])?),
        Value::Object(o) => (time(o.get("start")?)?, time(o.get("end")?)?),
        Value::String(s) => {
            let (a, b) = s.split_once('-')?;
            (parse_clock(a)?, parse_clock(b)?)
        }
        _ => return None,
    };
    Some(Segment { start, end })
}

fn time(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_clock(s),
        _ => None,
    }
}

/// The first JSON value in a text: inside a fence if there is one, else from the first
/// `[` or `{` to its matching end.
fn extract_json(text: &str) -> Option<Value> {
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let body = after.split_once('\n').map_or(after, |(_, b)| b);
        if let Some(end) = body.find("```")
            && let Ok(v) = serde_json::from_str(body[..end].trim())
        {
            return Some(v);
        }
    }
    let start = text.find(['[', '{'])?;
    let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    stream.next()?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_fenced_answer_with_clock_times() {
        let answer = r#"Here you go:
```json
{"moments": [{"segments": [["31:23", "31:43.5"], {"start": "32:12.5", "end": 1944}],
  "hook": "6 étoiles", "caption": "L'armée débarque #gta6", "kind": "payoff", "score": 9}]}
```"#;
        let m = parse(answer).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(
            m[0].segments[0],
            Segment {
                start: 1883.0,
                end: 1903.5
            }
        );
        assert_eq!(m[0].segments[1].end, 1944.0);
        assert_eq!(m[0].score, 9.0);
    }

    #[test]
    fn reads_parts_written_by_a_person() {
        let s = parse_segments("12:03-12:30, 13:10.5-13:20").unwrap();
        assert_eq!(
            s[1],
            Segment {
                start: 790.5,
                end: 800.0
            }
        );
        assert!(parse_segments("12:30-12:03").is_err());
        assert!(parse_segments("soon").is_err());
    }

    #[test]
    fn reads_a_bare_list_after_some_talk() {
        let m = parse(r#"Sure. [{"start": 10, "end": "00:30", "hook": "x"}] done"#).unwrap();
        assert_eq!(
            m[0].segments,
            vec![Segment {
                start: 10.0,
                end: 30.0
            }]
        );
    }

    #[test]
    fn validation_orders_merges_and_checks_length() {
        let m = Moment {
            segments: vec![
                Segment {
                    start: 40.0,
                    end: 50.0,
                },
                Segment {
                    start: 10.0,
                    end: 20.0,
                },
                Segment {
                    start: 19.9,
                    end: 25.0,
                },
                Segment {
                    start: 60.0,
                    end: 60.2,
                },
            ],
            hook: " Hook ".into(),
            caption: String::new(),
            kind: String::new(),
            why: String::new(),
            score: 0.0,
        };
        let v = m.clone().validated(100.0, 15.0, 40.0).unwrap();
        assert_eq!(
            v.segments,
            vec![
                Segment {
                    start: 10.0,
                    end: 25.0
                },
                Segment {
                    start: 40.0,
                    end: 50.0
                }
            ]
        );
        assert_eq!(v.hook, "Hook");
        assert!(m.validated(100.0, 40.0, 60.0).is_err());
    }

    #[test]
    fn overlapping_moments_are_the_same_moment() {
        let a = Moment {
            segments: vec![Segment {
                start: 0.0,
                end: 30.0,
            }],
            hook: "a".into(),
            caption: String::new(),
            kind: String::new(),
            why: String::new(),
            score: 0.0,
        };
        let mut b = a.clone();
        b.segments = vec![Segment {
            start: 10.0,
            end: 35.0,
        }];
        let mut c = a.clone();
        c.segments = vec![Segment {
            start: 28.0,
            end: 50.0,
        }];
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
    }
}
