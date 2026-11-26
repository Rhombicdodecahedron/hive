use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use hive_core::{Device, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryKind {
    Video,
    Image,
}

impl GalleryKind {
    fn dir(self) -> &'static str {
        match self {
            GalleryKind::Video => "/sdcard/Movies/hive",
            GalleryKind::Image => "/sdcard/Pictures/hive",
        }
    }
    fn uri(self) -> &'static str {
        match self {
            GalleryKind::Video => "content://media/external/video/media",
            GalleryKind::Image => "content://media/external/images/media",
        }
    }
}

/// Files pushed into the phone's gallery, to delete once posted.
#[derive(Debug, Clone, Default)]
pub struct Pushed {
    pub remote: Vec<String>,
}

const GALLERY_WAIT: Duration = Duration::from_secs(30);
/// Seconds between two pictures of a slideshow, so the gallery dates them apart and lists
/// them in order.
const GAP: Duration = Duration::from_millis(1100);

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Put `files` in the phone's gallery so TikTok's picker lists them, newest first: the last
/// file is pushed first, so the first one ends up in the picker's first cell. Every name is new
/// (`hive-<stamp>-01.mp4`), so the picker can never pick an older copy. Returns once the
/// gallery lists every file.
pub fn push_to_gallery(
    dev: &impl Device,
    kind: GalleryKind,
    files: &[&Path],
    stamp: &str,
) -> Result<Pushed> {
    dev.shell(&format!("mkdir -p {}", kind.dir()))?;
    let mut pushed = Pushed::default();
    let mut names = Vec::new();
    for (n, file) in files.iter().enumerate().rev() {
        if !file.is_file() {
            return Err(Error::Invalid(format!("{} isn't a file", file.display())));
        }
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_lowercase();
        let name = format!("hive-{stamp}-{:02}.{ext}", n + 1);
        let remote = format!("{}/{name}", kind.dir());
        dev.push(file, &remote)?;
        pushed.remote.push(remote.clone());
        // adb push keeps the computer's date: date it now, so the gallery sorts it as newest
        dev.shell(&format!(
            "touch {r}; am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d {u} >/dev/null",
            r = quote(&remote),
            u = quote(&format!("file://{remote}"))
        ))?;
        names.push(name);
        if n > 0 {
            thread::sleep(GAP);
        }
    }
    let deadline = Instant::now() + GALLERY_WAIT;
    loop {
        let listed = dev.shell(&format!(
            "content query --uri {} --projection _display_name --where \"_display_name LIKE 'hive-{stamp}-%'\"",
            kind.uri()
        ))?;
        if names.iter().all(|n| listed.contains(n.as_str())) {
            return Ok(pushed);
        }
        if Instant::now() >= deadline {
            return Err(Error::App(format!(
                "the files are on the phone ({}) but its gallery doesn't list them, so nothing was \
                 picked; open them once in the phone's Gallery or Files app, then retry",
                kind.dir()
            )));
        }
        thread::sleep(Duration::from_millis(700));
    }
}

/// Delete what [`push_to_gallery`] put on the phone. Best effort.
pub fn remove_from_gallery(dev: &impl Device, pushed: &Pushed) {
    for remote in &pushed.remote {
        if remote.starts_with("/sdcard/Movies/hive/")
            || remote.starts_with("/sdcard/Pictures/hive/")
        {
            let _ = dev.shell(&format!(
                "rm -f {r}; am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d {u} >/dev/null",
                r = quote(remote),
                u = quote(&format!("file://{remote}"))
            ));
        }
    }
}
