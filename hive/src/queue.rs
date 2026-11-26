//! Posts planned for a time: `queue.json`, run by `hive serve`.

use std::fs;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hive_core::Post;
use jiff::{Timestamp, Zoned, tz::TimeZone};
use serde::{Deserialize, Serialize};

use crate::home::{Home, read_json, write_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for its time (or for a phone).
    Pending,
    /// A phone is on it right now.
    Posting,
    Posted,
    /// Left on the phone for a person to finish (a problem along the way, or a draft).
    OnPhone,
    /// Failed twice: needs a look.
    Failed,
    /// Too late, or removed from the plan.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub at: Timestamp,
    pub post: Post,
    pub status: Status,
    #[serde(default)]
    pub tries: u32,
    /// Why it's waiting, what failed, or what's left to do.
    #[serde(default)]
    pub note: String,
    /// The phone that posted it.
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub done_at: Option<Timestamp>,
}

/// "2026-11-27 18:00" (this computer's time zone), or an RFC 3339 timestamp.
pub fn parse_time(raw: &str) -> Result<Timestamp> {
    let raw = raw.trim();
    if let Ok(t) = raw.parse::<Timestamp>() {
        return Ok(t);
    }
    let civil: jiff::civil::DateTime = raw
        .replacen(' ', "T", 1)
        .parse()
        .with_context(|| format!("not a time: {raw:?} (write \"2026-11-27 18:00\")"))?;
    Ok(civil.to_zoned(TimeZone::system())?.timestamp())
}

pub fn local(t: Timestamp) -> String {
    t.to_zoned(TimeZone::system())
        .strftime("%a %d %b %H:%M")
        .to_string()
}

/// The queue file, read and written under a lock file so `hive post --at` and `hive serve`
/// never lose each other's changes.
pub struct Queue<'h> {
    home: &'h Home,
}

struct FileLock(std::path::PathBuf);

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl<'h> Queue<'h> {
    pub fn new(home: &'h Home) -> Self {
        Self { home }
    }

    fn lock(&self) -> Result<FileLock> {
        let path = self.home.queue_file().with_extension("lock");
        fs::create_dir_all(&self.home.data_dir)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(FileLock(path)),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(_) => {
                    // a writer that died holding it: a write takes milliseconds
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        read_json(&self.home.queue_file())
    }

    /// Change the queue in one go.
    pub fn update<T>(&self, f: impl FnOnce(&mut Vec<Entry>) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        let mut entries: Vec<Entry> = read_json(&self.home.queue_file())?;
        let out = f(&mut entries)?;
        entries.sort_by_key(|e| e.at);
        write_json(&self.home.queue_file(), &entries)?;
        Ok(out)
    }

    pub fn add(&self, post: Post, at: Timestamp) -> Result<Entry> {
        let entry = Entry {
            id: format!("{}-{:04x}", at.as_second(), rand_u16()),
            at,
            post,
            status: Status::Pending,
            tries: 0,
            note: String::new(),
            phone: None,
            done_at: None,
        };
        self.update(|e| {
            e.push(entry.clone());
            Ok(())
        })?;
        Ok(entry)
    }

    pub fn remove(&self, id: &str) -> Result<Entry> {
        self.update(|entries| {
            let Some(i) = entries.iter().position(|e| e.id == id) else {
                bail!("no queued post {id}")
            };
            if entries[i].status == Status::Posting {
                bail!("{id} is being posted right now");
            }
            Ok(entries.remove(i))
        })
    }
}

fn rand_u16() -> u16 {
    (Zoned::now().timestamp().subsec_nanosecond() as u32 ^ std::process::id()) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_in_the_local_zone_or_absolute() {
        assert!(parse_time("2026-11-27 18:00").is_ok());
        assert_eq!(
            parse_time("2026-11-27T17:00:00Z").unwrap().as_second(),
            1_795_798_800
        );
        assert!(parse_time("tomorrow").is_err());
    }
}
