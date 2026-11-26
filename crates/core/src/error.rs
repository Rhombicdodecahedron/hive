use crate::{Handle, Serial};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What can go wrong, said so that someone who only plugged a phone in knows what to do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("adb isn't installed: install Android's platform-tools (adb) or set ANDROID_HOME")]
    NoAdb,
    #[error("adb failed: {0}")]
    Adb(String),
    #[error("phone {0} isn't plugged in (or USB debugging is off)")]
    PhoneGone(Serial),
    #[error("phone {serial} can't be used: {why}")]
    PhoneBlocked { serial: Serial, why: String },
    #[error("no phone can post for @{handle}: {why}")]
    NoPhone { handle: Handle, why: String },
    #[error("@{handle} isn't logged in to TikTok on {serial} (TikTok is on {current})")]
    NotOnPhone {
        handle: Handle,
        serial: Serial,
        current: String,
    },
    #[error("the phone's UI server: {0}")]
    Ui(String),
    #[error("TikTok: {0}")]
    App(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether waiting (for a phone to come back, to be unlocked, to be free) could fix it, as
    /// opposed to something wrong with the post or the app that a retry won't change.
    pub fn is_waiting(&self) -> bool {
        matches!(
            self,
            Error::PhoneGone(_) | Error::PhoneBlocked { .. } | Error::NoPhone { .. }
        )
    }
}
