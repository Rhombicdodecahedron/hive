//! Where hive keeps things, and its config.
//!
//! - config: `~/.config/hive/config.toml` (which phone posts for which account, the address of
//!   `hive serve`) and `tiktok.toml` (selector overrides, only the keys that changed);
//! - data: `~/.local/share/hive/` (phones.json, queue.json, locks, the UI server's APKs).
//!
//! `HIVE_HOME=<dir>` puts both in one folder, for a project that wants its own hive.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hive_core::{Handle, Serial};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone)]
pub struct Home {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountConfig {
    /// The phone that posts for this account; any phone holding it when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<Serial>,
}

fn default_address() -> String {
    "127.0.0.1:7878".into()
}

fn default_late() -> f64 {
    6.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Where `hive serve` listens. Keep it on 127.0.0.1: anyone who reaches it can post.
    #[serde(default = "default_address")]
    pub address: String,
    /// A queued post this many hours past its time is skipped instead of posted late.
    #[serde(default = "default_late")]
    pub late_limit_hours: f64,
    #[serde(default)]
    pub accounts: BTreeMap<Handle, AccountConfig>,
    /// `hive clips`' settings (kept as written when hive is built without the studio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio: Option<toml::Table>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: default_address(),
            late_limit_hours: default_late(),
            accounts: BTreeMap::new(),
            studio: None,
        }
    }
}

impl Home {
    pub fn locate() -> Result<Self> {
        if let Some(dir) = std::env::var_os("HIVE_HOME").map(PathBuf::from) {
            return Ok(Self {
                config_dir: dir.clone(),
                data_dir: dir,
            });
        }
        let config_dir = dirs::config_dir()
            .context("no config folder on this system")?
            .join("hive");
        let data_dir = dirs::data_dir()
            .context("no data folder on this system")?
            .join("hive");
        Ok(Self {
            config_dir,
            data_dir,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn selectors_file(&self) -> PathBuf {
        self.config_dir.join("tiktok.toml")
    }
    pub fn phones_file(&self) -> PathBuf {
        self.data_dir.join("phones.json")
    }
    pub fn queue_file(&self) -> PathBuf {
        self.data_dir.join("queue.json")
    }
    pub fn locks_dir(&self) -> PathBuf {
        self.data_dir.join("locks")
    }
    pub fn apks_dir(&self) -> PathBuf {
        self.data_dir.join("apks")
    }
    /// The studio's temporary files (audio, frames, subtitles, its font).
    #[cfg_attr(not(feature = "studio"), allow(dead_code))]
    pub fn studio_dir(&self) -> PathBuf {
        self.data_dir.join("studio")
    }

    pub fn config(&self) -> Result<Config> {
        let path = self.config_file();
        if !path.exists() {
            return Ok(Config::default());
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{} isn't valid", path.display()))
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        write_atomic(
            &self.config_file(),
            toml::to_string_pretty(config)?.as_bytes(),
        )
    }
}

/// Write through a temporary file and a rename, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} isn't valid", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}
