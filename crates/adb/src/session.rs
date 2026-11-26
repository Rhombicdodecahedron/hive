use std::thread;
use std::time::Duration;

use hive_core::{Device, Error, Lock, Result, parse_lock};

/// What to put back once the job is done.
#[derive(Debug, Clone, Default)]
pub struct Restore {
    stay_on: Option<String>,
}

fn lock(dev: &impl Device) -> Result<Lock> {
    let out = dev.shell(
        "dumpsys window policy | grep -E 'showing=|secure=|mIsSecure|Lockscreen=|isKeyguardShowing'; \
         echo @@; cmd lock_settings get-disabled",
    )?;
    let (policy, disabled) = out.split_once("@@").unwrap_or((&out, ""));
    Ok(parse_lock(policy, disabled))
}

fn screen_size(dev: &impl Device) -> (i32, i32) {
    // "Physical size: 1080x2400", then "Override size: …" when one is set: the last one counts
    let out = dev.shell("wm size").unwrap_or_default();
    out.lines()
        .filter_map(|l| l.rsplit(' ').next()?.split_once('x'))
        .filter_map(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
        .next_back()
        .unwrap_or((1080, 2400))
}

/// Get a phone ready for a post: screen on, lock screen out of the way, kept awake while the
/// job runs (a 2 min post outlasts most screen timeouts). Returns what [`rest`] puts back.
/// Fails when the lock screen needs a code: hive never types one.
pub fn open_up(dev: &impl Device) -> Result<Restore> {
    let stay_on = dev
        .shell("settings get global stay_on_while_plugged_in")?
        .trim()
        .to_owned();
    let restore = Restore {
        stay_on: (!stay_on.is_empty() && stay_on != "null").then_some(stay_on),
    };
    dev.shell("settings put global stay_on_while_plugged_in 7; input keyevent KEYCODE_WAKEUP")?;
    thread::sleep(Duration::from_millis(700));
    let l = lock(dev)?;
    if l.showing != Some(true) {
        return Ok(restore);
    }
    if l.secure == Some(true) {
        rest(dev, &restore);
        return Err(Error::PhoneBlocked {
            serial: dev.serial().clone(),
            why: "its lock screen needs a code: set the screen lock to None or Swipe".into(),
        });
    }
    dev.shell("wm dismiss-keyguard")?;
    thread::sleep(Duration::from_millis(700));
    if lock(dev)?.showing == Some(true) {
        let (w, h) = screen_size(dev);
        dev.shell(&format!(
            "input swipe {} {} {} {} 250",
            w / 2,
            h * 4 / 5,
            w / 2,
            h / 5
        ))?;
        thread::sleep(Duration::from_millis(700));
    }
    if lock(dev)?.showing == Some(true) {
        rest(dev, &restore);
        return Err(Error::PhoneBlocked {
            serial: dev.serial().clone(),
            why: "its lock screen didn't open: unlock it once by hand and check it doesn't ask for a code".into(),
        });
    }
    Ok(restore)
}

/// Back to rest: the phone's own "stay awake" setting, the home screen, the screen off.
/// Best effort.
pub fn rest(dev: &impl Device, restore: &Restore) {
    let stay_on = restore.stay_on.as_deref().unwrap_or("0");
    let _ = dev.shell(&format!(
        "settings put global stay_on_while_plugged_in {stay_on}; input keyevent KEYCODE_HOME; \
         input keyevent KEYCODE_SLEEP"
    ));
}
