use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Error;

/// A phone's adb serial ("R5CR31QL2GK", or "192.168.1.20:5555" over Wi-Fi).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Serial(String);

impl Serial {
    pub fn new(raw: &str) -> Result<Self, Error> {
        let ok = !raw.is_empty()
            && raw.len() <= 64
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c));
        if ok {
            Ok(Self(raw.to_owned()))
        } else {
            Err(Error::Invalid(format!("not a phone serial: {raw:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Connected over the network (`adb connect`) rather than USB.
    pub fn is_wireless(&self) -> bool {
        self.0.contains(':')
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Serial {
    type Error = Error;
    fn try_from(s: String) -> Result<Self, Error> {
        Serial::new(&s)
    }
}

impl From<Serial> for String {
    fn from(s: Serial) -> String {
        s.0
    }
}

impl std::str::FromStr for Serial {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Serial::new(s)
    }
}

/// The lock screen, as the keyguard reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// The lock screen is up.
    pub showing: Option<bool>,
    /// Opening it takes a PIN, pattern, password or fingerprint.
    pub secure: Option<bool>,
}

/// What one adb round trip says about a phone (see [`STATUS_SCRIPT`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub android: String,
    pub battery: Option<u8>,
    pub charging: bool,
    pub awake: Option<bool>,
    pub lock: Lock,
    /// TikTok's package, when installed.
    pub tiktok: Option<String>,
    /// TikTok is the app on screen.
    pub in_tiktok: bool,
    /// The UiAutomator2 server hive drives the phone with is installed.
    pub ui_server: bool,
}

/// A phone as adb lists it, with its status when it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phone {
    pub serial: Serial,
    /// adb's word for it: "device" (usable), "unauthorized", "offline"…
    pub state: String,
    pub model: String,
    pub status: Option<Status>,
}

/// Why a phone can't post right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Blocker {
    Unauthorized,
    Offline,
    AdbState { state: String },
    NoTikTok,
    LockedWithCode,
    LowBattery { percent: u8 },
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blocker::Unauthorized => write!(f, "allow USB debugging on the phone's screen"),
            Blocker::Offline => write!(f, "offline: unplug it and plug it back in"),
            Blocker::AdbState { state } => write!(f, "adb sees it as {state:?}"),
            Blocker::NoTikTok => write!(f, "TikTok isn't installed"),
            Blocker::LockedWithCode => write!(
                f,
                "locked with a code: unlock it, or set its screen lock to None or Swipe so \
                 hive can open it by itself"
            ),
            Blocker::LowBattery { percent } => write!(f, "battery at {percent}% and not charging"),
        }
    }
}

/// Under this, a phone off its charger isn't used.
pub const LOW_BATTERY: u8 = 15;

impl Phone {
    pub fn label(&self) -> &str {
        if self.model.is_empty() {
            self.serial.as_str()
        } else {
            &self.model
        }
    }

    pub fn blocker(&self) -> Option<Blocker> {
        match self.state.as_str() {
            "device" => {}
            "unauthorized" => return Some(Blocker::Unauthorized),
            "offline" => return Some(Blocker::Offline),
            other => {
                return Some(Blocker::AdbState {
                    state: other.to_owned(),
                });
            }
        }
        let s = self.status.as_ref()?;
        if s.tiktok.is_none() {
            return Some(Blocker::NoTikTok);
        }
        // unlocked by hand a moment ago, a phone with a code can still post
        if s.lock.secure == Some(true) && s.lock.showing != Some(false) {
            return Some(Blocker::LockedWithCode);
        }
        match s.battery {
            Some(p) if p < LOW_BATTERY && !s.charging => Some(Blocker::LowBattery { percent: p }),
            _ => None,
        }
    }
}

/// `adb devices -l` → the phones it lists.
pub fn parse_devices(out: &str) -> Vec<Phone> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = Serial::new(parts.next()?).ok()?;
            let state = parts.next()?.to_owned();
            let model = line
                .split_whitespace()
                .find_map(|p| p.strip_prefix("model:"))
                .unwrap_or("")
                .replace('_', " ");
            Some(Phone {
                serial,
                state,
                model,
                status: None,
            })
        })
        .collect()
}

/// Everything [`Status`] holds, in ONE `adb shell` (a full `dumpsys window` is megabytes and
/// takes seconds, so each part is filtered on the phone). Sections are split by "@@".
pub const STATUS_SCRIPT: &str = "getprop ro.build.version.release; echo @@; \
dumpsys battery | grep -E 'level:|powered:'; echo @@; \
dumpsys power | grep -E 'mWakefulness='; echo @@; \
dumpsys window policy | grep -E 'showing=|secure=|mIsSecure|Lockscreen=|isKeyguardShowing'; echo @@; \
dumpsys window | grep -E 'mCurrentFocus='; echo @@; \
pm path com.zhiliaoapp.musically; pm path com.ss.android.ugc.trill; echo @@; \
cmd lock_settings get-disabled; echo @@; \
pm path io.appium.uiautomator2.server; pm path io.appium.uiautomator2.server.test";

pub const TIKTOK_PACKAGES: [&str; 2] = ["com.zhiliaoapp.musically", "com.ss.android.ugc.trill"];

fn flag(text: &str, key: &str) -> Option<bool> {
    let at = text.match_indices(key).find(|(i, _)| {
        // a whole key: "showing=" but not "mShowing=" or "isShowing="
        *i == 0 || !text.as_bytes()[i - 1].is_ascii_alphanumeric()
    })?;
    let rest = &text[at.0 + key.len()..];
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// The lock screen from `dumpsys window policy` (and `cmd lock_settings get-disabled`).
pub fn parse_lock(policy: &str, disabled: &str) -> Lock {
    let showing = flag(policy, "showing=")
        .or_else(|| flag(policy, "mShowingLockscreen="))
        .or_else(|| flag(policy, "isKeyguardShowing="))
        .or_else(|| flag(policy, "mDreamingLockscreen="));
    let mut secure = flag(policy, "secure=").or_else(|| flag(policy, "mIsSecure="));
    if secure.is_none() && disabled.trim() == "true" {
        secure = Some(false); // no lock screen at all
    }
    Lock { showing, secure }
}

pub fn parse_status(out: &str) -> Status {
    let mut parts: Vec<&str> = out.split("@@").collect();
    parts.resize(8, "");
    let [
        android,
        battery,
        power,
        policy,
        focus,
        paths,
        disabled,
        server,
    ] = [
        parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6], parts[7],
    ];
    let level = battery
        .lines()
        .find_map(|l| l.trim().strip_prefix("level:"))
        .and_then(|v| v.trim().parse::<u8>().ok());
    let charging = battery.lines().any(|l| {
        let l = l.trim();
        l.contains("powered:") && l.ends_with("true")
    });
    let awake = power
        .split("mWakefulness=")
        .nth(1)
        .map(|v| v.trim_start().starts_with("Awake"));
    let tiktok = TIKTOK_PACKAGES
        .iter()
        .find(|p| {
            paths
                .lines()
                .any(|l| l.starts_with("package:") && l.contains(*p))
        })
        .map(|p| (*p).to_owned());
    Status {
        android: android.trim().to_owned(),
        battery: level,
        charging,
        awake,
        lock: parse_lock(policy, disabled),
        in_tiktok: TIKTOK_PACKAGES.iter().any(|p| focus.contains(p)),
        tiktok,
        ui_server: server.lines().filter(|l| l.starts_with("package:")).count() >= 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_phones_with_their_model() {
        let out = "* daemon started successfully\nList of devices attached\n\
                   R5CR31QL2GK            device usb:1-2 product:o1sxeea model:SM_G991B device:o1s transport_id:3\n\
                   192.168.1.20:5555      unauthorized transport_id:4\n\n";
        let phones = parse_devices(out);
        assert_eq!(phones.len(), 2);
        assert_eq!(phones[0].model, "SM G991B");
        assert_eq!(phones[0].state, "device");
        assert!(phones[1].serial.is_wireless());
        assert_eq!(phones[1].blocker(), Some(Blocker::Unauthorized));
    }

    #[test]
    fn reads_a_status_in_one_go() {
        let out = "15\n@@\n  USB powered: true\n  level: 98\n@@\n  mWakefulness=Asleep\n@@\n\
                   KeyguardServiceDelegate\n    showing=true\n    showingAndNotOccluded=true\n    secure=false\n@@\n\
                   mCurrentFocus=Window{1 u0 NotificationShade}\n@@\n\
                   package:/data/app/~~a==/com.zhiliaoapp.musically-b==/base.apk\n@@\nfalse\n@@\n\
                   package:/data/app/~~c==/io.appium.uiautomator2.server-d==/base.apk\n\
                   package:/data/app/~~e==/io.appium.uiautomator2.server.test-f==/base.apk\n";
        let s = parse_status(out);
        assert_eq!(s.android, "15");
        assert_eq!(s.battery, Some(98));
        assert!(s.charging);
        assert_eq!(s.awake, Some(false));
        assert_eq!(
            s.lock,
            Lock {
                showing: Some(true),
                secure: Some(false)
            }
        );
        assert_eq!(s.tiktok.as_deref(), Some("com.zhiliaoapp.musically"));
        assert!(!s.in_tiktok);
        assert!(s.ui_server);
        let phone = Phone {
            serial: Serial::new("R5CR31QL2GK").unwrap(),
            state: "device".into(),
            model: "SM G991B".into(),
            status: Some(s),
        };
        assert_eq!(phone.blocker(), None);
    }

    #[test]
    fn a_code_blocks_only_while_the_lock_screen_is_up() {
        let mut phone = Phone {
            serial: Serial::new("A1").unwrap(),
            state: "device".into(),
            model: String::new(),
            status: Some(Status {
                tiktok: Some("com.zhiliaoapp.musically".into()),
                lock: Lock {
                    showing: Some(true),
                    secure: Some(true),
                },
                ..Status::default()
            }),
        };
        assert_eq!(phone.blocker(), Some(Blocker::LockedWithCode));
        phone.status.as_mut().unwrap().lock.showing = Some(false);
        assert_eq!(phone.blocker(), None);
    }

    #[test]
    fn whole_keys_only() {
        assert_eq!(flag("mShowing=true showing=false", "showing="), Some(false));
        assert_eq!(parse_lock("", "true").secure, Some(false));
    }
}
