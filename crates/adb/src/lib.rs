//! Phones over adb: which ones are plugged in and how they are, a shell on each, pushing
//! files into the gallery, waking a phone up for a post and putting it back to rest.
//!
//! It runs the `adb` program (Android's platform-tools): adb's own server already owns the USB
//! side, so going through it means hive never fights `adb` or Android Studio for a phone.

mod command;
mod gallery;
mod session;

use std::path::PathBuf;
use std::thread;

use hive_core::{Device, Error, Phone, Result, STATUS_SCRIPT, Serial, parse_devices, parse_status};

pub use command::AdbDevice;
pub use gallery::{GalleryKind, Pushed, push_to_gallery, remove_from_gallery};
pub use session::{Restore, open_up, rest};

/// The adb program.
#[derive(Debug, Clone)]
pub struct Adb {
    exe: PathBuf,
}

impl Adb {
    /// adb on the PATH, else in the Android SDK ($ANDROID_HOME, $ANDROID_SDK_ROOT, the usual
    /// install folders).
    pub fn find() -> Result<Self> {
        if let Some(exe) = std::env::var_os("HIVE_ADB").map(PathBuf::from) {
            return Ok(Self { exe });
        }
        let name = if cfg!(windows) { "adb.exe" } else { "adb" };
        let on_path = std::env::var_os("PATH")
            .map(|p| {
                std::env::split_paths(&p)
                    .map(|d| d.join(name))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let home = dirs_home();
        let sdks = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .chain(home.iter().flat_map(|h| {
                [
                    h.join("Android/Sdk"),
                    h.join("Library/Android/sdk"),
                    h.join("AppData/Local/Android/Sdk"),
                ]
            }))
            .map(|sdk| sdk.join("platform-tools").join(name));
        on_path
            .into_iter()
            .chain(sdks)
            .find(|p| p.is_file())
            .map(|exe| Self { exe })
            .ok_or(Error::NoAdb)
    }

    pub fn with_exe(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into() }
    }

    pub fn exe(&self) -> &PathBuf {
        &self.exe
    }

    /// Every phone adb lists, with its status read in one round trip each, all phones at once.
    pub fn phones(&self) -> Result<Vec<Phone>> {
        let out = command::run(&self.exe, &["devices", "-l"], command::QUICK)?;
        let mut phones = parse_devices(&out);
        thread::scope(|s| {
            for p in phones.iter_mut().filter(|p| p.state == "device") {
                s.spawn(move || {
                    let dev = self.device(p.serial.clone());
                    if let Ok(out) = dev.shell(STATUS_SCRIPT) {
                        p.status = Some(parse_status(&out));
                    }
                });
            }
        });
        Ok(phones)
    }

    pub fn device(&self, serial: Serial) -> AdbDevice {
        AdbDevice::new(self.exe.clone(), serial)
    }

    /// Connect to a phone over Wi-Fi ("192.168.1.20:5555"), after `adb tcpip` or Android's
    /// wireless debugging pairing.
    pub fn connect(&self, address: &str) -> Result<String> {
        command::run(&self.exe, &["connect", address], command::QUICK)
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
