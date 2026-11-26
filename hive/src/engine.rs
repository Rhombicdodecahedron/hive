//! One post, start to end, on whichever phone holds the account.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use hive_adb::{Adb, AdbDevice, GalleryKind, open_up, push_to_gallery, remove_from_gallery, rest};
use hive_core::{
    Device, Error, Handle, Media, Phone, PhoneMemory, PhoneNotes, Post, PostReport, Serial,
    choose_phones,
};
use hive_tiktok::{App, TikTok};
use hive_uia2::{Session, ensure_installed};
use jiff::Zoned;
use serde::Serialize;

use crate::home::{Config, Home, read_json, write_json};

/// A phone as hive shows it: adb's view, what hive remembers, whether it is posting now.
#[derive(Debug, Clone, Serialize)]
pub struct PhoneView {
    #[serde(flatten)]
    pub phone: Phone,
    pub blocker: Option<String>,
    pub busy: bool,
    pub notes: PhoneNotes,
    /// Accounts set to post from this phone in the config.
    pub bound: Vec<Handle>,
}

/// A post that went through on a phone (or was left on it for a person to finish).
#[derive(Debug, Clone, Serialize)]
pub struct Posted {
    pub phone: Serial,
    pub report: PostReport,
}

pub struct Hive {
    pub home: Home,
    pub adb: Adb,
    pub app: App,
    memory: Mutex<()>,
}

fn now() -> String {
    Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// A phone being driven by this process: the file says so to every other hive process.
struct PhoneLock(PathBuf);

impl PhoneLock {
    fn path(home: &Home, serial: &Serial) -> PathBuf {
        home.locks_dir()
            .join(format!("{}.lock", serial.as_str().replace(':', "_")))
    }

    fn holder_alive(path: &PathBuf) -> bool {
        let Ok(text) = fs::read_to_string(path) else {
            return false;
        };
        let pid = text.trim();
        if cfg!(target_os = "linux") {
            return PathBuf::from(format!("/proc/{pid}")).exists();
        }
        // elsewhere: trust a lock for 30 minutes
        fs::metadata(path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| {
                SystemTime::now().duration_since(t).unwrap_or_default() < Duration::from_secs(1800)
            })
    }

    fn busy(home: &Home, serial: &Serial) -> bool {
        let p = Self::path(home, serial);
        p.exists() && Self::holder_alive(&p)
    }

    fn take(home: &Home, serial: &Serial) -> Option<Self> {
        let p = Self::path(home, serial);
        fs::create_dir_all(home.locks_dir()).ok()?;
        if p.exists() && !Self::holder_alive(&p) {
            let _ = fs::remove_file(&p);
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .ok()?;
        use std::io::Write;
        let _ = write!(f, "{}", std::process::id());
        Some(Self(p))
    }
}

impl Drop for PhoneLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl Hive {
    pub fn open() -> Result<Self> {
        let home = Home::locate()?;
        let app = App::load(Some(&home.selectors_file()))?;
        Ok(Self {
            adb: Adb::find()?,
            app,
            home,
            memory: Mutex::new(()),
        })
    }

    pub fn config(&self) -> Result<Config> {
        self.home.config()
    }

    pub fn memory(&self) -> PhoneMemory {
        read_json(&self.home.phones_file()).unwrap_or_default()
    }

    fn remember(&self, f: impl FnOnce(&mut PhoneMemory)) {
        let _guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.memory();
        f(&mut m);
        let _ = write_json(&self.home.phones_file(), &m);
    }

    pub fn phones(&self) -> Result<Vec<PhoneView>> {
        let config = self.config()?;
        let memory = self.memory();
        Ok(self
            .adb
            .phones()?
            .into_iter()
            .map(|phone| PhoneView {
                blocker: phone.blocker().map(|b| b.to_string()),
                busy: PhoneLock::busy(&self.home, &phone.serial),
                notes: memory.get(&phone.serial).cloned().unwrap_or_default(),
                bound: config
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.phone.as_ref() == Some(&phone.serial))
                    .map(|(h, _)| h.clone())
                    .collect(),
                phone,
            })
            .collect())
    }

    pub fn device(&self, serial: &Serial) -> AdbDevice {
        self.adb.device(serial.clone())
    }

    /// Make `post` on a phone that holds its account (only `only` when given). Errors with
    /// `NoPhone` when no phone could even try: plugged out, locked with a code, busy.
    pub fn post(
        &self,
        post: &Post,
        only: Option<&Serial>,
        say: &mut dyn FnMut(&str),
    ) -> Result<Posted, Error> {
        for f in post.media.files() {
            if !f.is_file() {
                return Err(Error::Invalid(format!("{} isn't a file", f.display())));
            }
        }
        let config = self.config().map_err(|e| Error::Invalid(e.to_string()))?;
        let mut phones = self.adb.phones()?;
        if let Some(only) = only {
            phones.retain(|p| &p.serial == only);
        }
        let busy: Vec<Serial> = phones
            .iter()
            .filter(|p| PhoneLock::busy(&self.home, &p.serial))
            .map(|p| p.serial.clone())
            .collect();
        let bound = config
            .accounts
            .get(&post.account)
            .and_then(|a| a.phone.as_ref());
        let choice = choose_phones(
            &post.account,
            &phones,
            bound.or(only),
            &self.memory(),
            &busy,
        );
        let mut skipped = choice.skipped;
        for serial in choice.phones {
            let label = phones
                .iter()
                .find(|p| p.serial == serial)
                .map(|p| p.label().to_owned())
                .unwrap_or_default();
            let Some(_lock) = PhoneLock::take(&self.home, &serial) else {
                skipped.push(format!("{label}: busy posting"));
                continue;
            };
            say(&format!("{label} ({serial}): waking it up"));
            let dev = self.device(&serial);
            let restore = match open_up(&dev) {
                Ok(r) => r,
                Err(e) if e.is_waiting() => {
                    skipped.push(format!("{label}: {e}"));
                    continue;
                }
                Err(e) => return Err(e),
            };
            self.remember(|m| m.set_model(&serial, &label));
            match self.post_on(&dev, post, say) {
                Ok((report, pushed)) => {
                    if report.posted {
                        self.remember(|m| m.posted(&serial, &post.account, &now()));
                        remove_from_gallery(&dev, &pushed);
                        rest(&dev, &restore);
                    } else {
                        // left on the post screen for a person: keep the files and the screen
                        self.remember(|m| m.has(&serial, &post.account, &now()));
                    }
                    return Ok(Posted {
                        phone: serial,
                        report,
                    });
                }
                Err((Error::NotOnPhone { current, .. }, pushed)) => {
                    say(&format!(
                        "@{} isn't on {label} (TikTok is on {current}): next phone",
                        post.account
                    ));
                    let current = Handle::new(&current).ok();
                    self.remember(|m| m.lacks(&serial, &post.account, current.as_ref(), &now()));
                    remove_from_gallery(&dev, &pushed);
                    rest(&dev, &restore);
                    skipped.push(format!(
                        "{label}: @{} isn't logged in to its TikTok",
                        post.account
                    ));
                }
                Err((e, pushed)) => {
                    remove_from_gallery(&dev, &pushed);
                    rest(&dev, &restore);
                    if matches!(e, Error::PhoneGone(_)) {
                        skipped.push(format!("{label}: unplugged during the post"));
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(Error::NoPhone {
            handle: post.account.clone(),
            why: if skipped.is_empty() {
                "no phone plugged in".into()
            } else {
                skipped.join("; ")
            },
        })
    }

    /// The post on one phone that is awake and open: UI server, account, files, TikTok.
    fn post_on(
        &self,
        dev: &AdbDevice,
        post: &Post,
        say: &mut dyn FnMut(&str),
    ) -> Result<(PostReport, hive_adb::Pushed), (Error, hive_adb::Pushed)> {
        let none = hive_adb::Pushed::default();
        match ensure_installed(dev, &self.home.apks_dir()) {
            Ok(true) => say("installed hive's UI server on the phone"),
            Ok(false) => {}
            Err(e) => return Err((e, none)),
        }
        say("starting the UI server");
        let mut session = Session::start(dev.clone()).map_err(|e| (e, none.clone()))?;
        let mut tiktok = TikTok::new(&mut session, dev, &self.app, say);
        tiktok
            .open_on(&post.account)
            .map_err(|e| (e, none.clone()))?;
        let stamp = Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let (kind, files): (GalleryKind, Vec<&std::path::Path>) = match &post.media {
            Media::Video { file } => (GalleryKind::Video, vec![file.as_path()]),
            Media::Slideshow { images } => (
                GalleryKind::Image,
                images.iter().map(PathBuf::as_path).collect(),
            ),
        };
        let pushed = push_to_gallery(dev, kind, &files, &stamp).map_err(|e| (e, none.clone()))?;
        let mut tiktok = TikTok::new(&mut session, dev, &self.app, say);
        match tiktok.publish_here(post) {
            Ok(report) => Ok((report, pushed)),
            Err(e) => Err((e, pushed)),
        }
    }

    /// Wake the phone, open TikTok and read the account it is on.
    pub fn check(&self, serial: &Serial, say: &mut dyn FnMut(&str)) -> Result<Handle, Error> {
        let _lock = PhoneLock::take(&self.home, serial).ok_or_else(|| Error::PhoneBlocked {
            serial: serial.clone(),
            why: "busy posting".into(),
        })?;
        let dev = self.device(serial);
        let restore = open_up(&dev)?;
        let result = (|| {
            if ensure_installed(&dev, &self.home.apks_dir())? {
                say("installed hive's UI server on the phone");
            }
            let mut session = Session::start(dev.clone())?;
            TikTok::new(&mut session, &dev, &self.app, say).current_account()
        })();
        rest(&dev, &restore);
        let handle = result?;
        self.remember(|m| m.has(serial, &handle, &now()));
        Ok(handle)
    }

    /// Install the UI server on the phone. True when it was missing.
    pub fn setup(&self, serial: &Serial) -> Result<bool, Error> {
        ensure_installed(&self.device(serial), &self.home.apks_dir())
    }

    /// The screen's elements as XML (to fix a selector).
    pub fn screen_xml(&self, serial: &Serial) -> Result<String, Error> {
        let _lock = PhoneLock::take(&self.home, serial).ok_or_else(|| Error::PhoneBlocked {
            serial: serial.clone(),
            why: "busy posting".into(),
        })?;
        let dev = self.device(serial);
        ensure_installed(&dev, &self.home.apks_dir())?;
        let mut session = Session::start(dev.clone())?;
        hive_core::Ui::source(&mut session)
    }

    /// The default phone when only one is plugged in and usable.
    pub fn only_phone(&self) -> Result<Serial, Error> {
        let phones = self.adb.phones()?;
        match phones
            .iter()
            .filter(|p| p.state == "device")
            .collect::<Vec<_>>()
            .as_slice()
        {
            [one] => Ok(one.serial.clone()),
            [] => Err(Error::Invalid(
                "no phone plugged in (adb devices lists none)".into(),
            )),
            _ => Err(Error::Invalid(
                "several phones are plugged in: say which with --phone".into(),
            )),
        }
    }

    pub fn wake(&self, serial: &Serial) -> Result<(), Error> {
        self.device(serial)
            .shell("input keyevent KEYCODE_WAKEUP")
            .map(drop)
    }

    pub fn sleep(&self, serial: &Serial) -> Result<(), Error> {
        self.device(serial)
            .shell("input keyevent KEYCODE_SLEEP")
            .map(drop)
    }
}
