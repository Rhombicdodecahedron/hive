use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Error;

/// A TikTok account's handle, the way TikTok compares them: no "@", lowercase.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Handle(String);

impl Handle {
    pub fn new(raw: &str) -> Result<Self, Error> {
        let h = raw.trim().trim_start_matches('@').to_lowercase();
        let ok = (2..=24).contains(&h.len())
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
        if ok {
            Ok(Self(h))
        } else {
            Err(Error::Invalid(format!("not a TikTok handle: {raw:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Handle {
    type Error = Error;
    fn try_from(s: String) -> Result<Self, Error> {
        Handle::new(&s)
    }
}

impl From<Handle> for String {
    fn from(h: Handle) -> String {
        h.0
    }
}

impl std::str::FromStr for Handle {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Handle::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_compare_like_tiktok() {
        assert_eq!(
            Handle::new("@MomentsDeGTA6").unwrap().as_str(),
            "momentsdegta6"
        );
        assert_eq!(
            Handle::new(" gta6.clips_fr ").unwrap().as_str(),
            "gta6.clips_fr"
        );
        assert!(Handle::new("a").is_err());
        assert!(Handle::new("bad handle").is_err());
    }
}
