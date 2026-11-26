use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Handle, Serial};

/// What hive has seen on one phone. Only a hint for picking phones: TikTok has the last word.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneNotes {
    #[serde(default)]
    pub model: String,
    /// Accounts seen in this phone's TikTok, with when (RFC 3339).
    #[serde(default)]
    pub accounts: BTreeMap<Handle, String>,
    /// Accounts this phone's TikTok was found not to hold, with when.
    #[serde(default)]
    pub missing: BTreeMap<Handle, String>,
    /// The account TikTok was on the last time hive looked.
    #[serde(default)]
    pub current: Option<Handle>,
    /// "2026-11-26T20:09:00+01:00 @account"
    #[serde(default)]
    pub last_post: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhoneMemory(pub BTreeMap<Serial, PhoneNotes>);

impl PhoneMemory {
    pub fn get(&self, serial: &Serial) -> Option<&PhoneNotes> {
        self.0.get(serial)
    }

    /// TikTok on `serial` holds `handle` (it was on it, or switched to it).
    pub fn has(&mut self, serial: &Serial, handle: &Handle, now: &str) {
        let n = self.0.entry(serial.clone()).or_default();
        n.accounts.insert(handle.clone(), now.to_owned());
        n.missing.remove(handle);
        n.current = Some(handle.clone());
    }

    /// TikTok on `serial` doesn't hold `handle`; it was on `current`.
    pub fn lacks(&mut self, serial: &Serial, handle: &Handle, current: Option<&Handle>, now: &str) {
        let n = self.0.entry(serial.clone()).or_default();
        n.missing.insert(handle.clone(), now.to_owned());
        n.accounts.remove(handle);
        if let Some(c) = current {
            n.accounts.insert(c.clone(), now.to_owned());
            n.current = Some(c.clone());
        }
    }

    pub fn posted(&mut self, serial: &Serial, handle: &Handle, now: &str) {
        self.has(serial, handle, now);
        self.0.entry(serial.clone()).or_default().last_post = Some(format!("{now} @{handle}"));
    }

    pub fn set_model(&mut self, serial: &Serial, model: &str) {
        if !model.is_empty() {
            self.0.entry(serial.clone()).or_default().model = model.to_owned();
        }
    }
}
