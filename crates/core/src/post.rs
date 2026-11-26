use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Handle};

/// What goes in a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Media {
    /// One video file.
    Video { file: PathBuf },
    /// Pictures shown one after the other, in this order (TikTok's photo mode).
    Slideshow { images: Vec<PathBuf> },
}

impl Media {
    /// A video for one file with a video extension, a slideshow for pictures.
    pub fn from_files(files: Vec<PathBuf>) -> Result<Self, Error> {
        const VIDEO: [&str; 5] = ["mp4", "mov", "m4v", "webm", "mkv"];
        const IMAGE: [&str; 5] = ["jpg", "jpeg", "png", "webp", "heic"];
        let ext = |p: &PathBuf| {
            p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default()
        };
        match files.as_slice() {
            [] => Err(Error::Invalid("nothing to post: give a video or pictures".into())),
            [one] if VIDEO.contains(&ext(one).as_str()) => Ok(Media::Video { file: one.clone() }),
            many if many.iter().all(|f| IMAGE.contains(&ext(f).as_str())) => {
                if many.len() > 35 {
                    Err(Error::Invalid("TikTok takes at most 35 pictures in a post".into()))
                } else {
                    Ok(Media::Slideshow { images: many.to_vec() })
                }
            }
            _ => Err(Error::Invalid(
                "post one video (mp4, mov…) or pictures (jpg, png, webp…), not a mix".into(),
            )),
        }
    }

    pub fn files(&self) -> Vec<&PathBuf> {
        match self {
            Media::Video { file } => vec![file],
            Media::Slideshow { images } => images.iter().collect(),
        }
    }

    pub fn count(&self) -> usize {
        self.files().len()
    }
}

/// Who can see a post, in the order TikTok lists them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Everyone,
    Friends,
    OnlyYou,
}

impl std::str::FromStr for Visibility {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_lowercase().replace(['-', ' '], "_").as_str() {
            "everyone" | "public" => Ok(Visibility::Everyone),
            "friends" => Ok(Visibility::Friends),
            "only_you" | "private" | "me" => Ok(Visibility::OnlyYou),
            _ => Err(Error::Invalid(format!("visibility: everyone, friends or only-you, not {s:?}"))),
        }
    }
}

/// A post to make on one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub account: Handle,
    pub media: Media,
    /// The description, #hashtags included.
    #[serde(default)]
    pub caption: String,
    /// A slideshow's title (TikTok shows a title box for photo posts).
    #[serde(default)]
    pub title: Option<String>,
    /// Search TikTok's sounds for this and use the first result.
    #[serde(default)]
    pub sound: Option<String>,
    #[serde(default)]
    pub visibility: Visibility,
    /// Fill everything in but leave Post for a person to tap.
    #[serde(default)]
    pub draft: bool,
}

/// What a post came to. `problems` are steps a person has to finish on the phone; with any, Post
/// isn't tapped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostReport {
    pub posted: bool,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    /// The sound TikTok used, as it names it.
    pub sound: Option<String>,
    pub visibility: Option<Visibility>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_or_slideshow_from_the_files() {
        let v = Media::from_files(vec!["clip.MP4".into()]).unwrap();
        assert!(matches!(v, Media::Video { .. }));
        let s = Media::from_files(vec!["1.jpg".into(), "2.png".into()]).unwrap();
        assert_eq!(s.count(), 2);
        assert!(Media::from_files(vec!["a.mp4".into(), "b.jpg".into()]).is_err());
        assert!(Media::from_files(vec![]).is_err());
    }
}
