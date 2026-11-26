//! hive-adb against a fake `adb` (a shell script), so the tests run without a phone.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use hive_adb::{Adb, open_up, rest};
use hive_core::{Blocker, Serial};

const FAKE: &str = r#"#!/bin/sh
log="$(dirname "$0")/calls.log"
echo "$*" >> "$log"
case "$*" in
  "devices -l") printf 'List of devices attached\nPHONE1 device usb:1 model:Pixel_8 device:x\nPHONE2 unauthorized usb:2\n' ;;
  *"getprop ro.build"*) printf '15\n@@\n  USB powered: true\n  level: 71\n@@\n  mWakefulness=Asleep\n@@\n    showing=true\n    secure=false\n@@\nmCurrentFocus=Window{launcher}\n@@\npackage:/data/app/com.zhiliaoapp.musically-1/base.apk\n@@\nfalse\n@@\n' ;;
  *"settings get global stay_on_while_plugged_in"*) echo 0 ;;
  *"dumpsys window policy"*)
     if grep -q "dismiss-keyguard" "$log"; then printf '    showing=false\n    secure=false\n@@\nfalse\n'; else printf '    showing=true\n    secure=false\n@@\nfalse\n'; fi ;;
  *) ;;
esac
"#;

fn fake_adb() -> (PathBuf, Adb) {
    let dir = std::env::temp_dir().join(format!("hive-adb-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("adb");
    fs::write(&exe, FAKE).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    let _ = fs::remove_file(dir.join("calls.log"));
    (dir, Adb::with_exe(exe))
}

#[test]
fn lists_phones_and_opens_one_up() {
    let (dir, adb) = fake_adb();
    let phones = adb.phones().unwrap();
    assert_eq!(phones.len(), 2);
    let p = &phones[0];
    assert_eq!(p.model, "Pixel 8");
    let s = p.status.as_ref().unwrap();
    assert_eq!(
        (s.battery, s.charging, s.awake),
        (Some(71), true, Some(false))
    );
    assert_eq!(p.blocker(), None);
    assert_eq!(phones[1].blocker(), Some(Blocker::Unauthorized));

    let dev = adb.device(Serial::new("PHONE1").unwrap());
    let restore = open_up(&dev).unwrap();
    rest(&dev, &restore);
    let calls = fs::read_to_string(dir.join("calls.log")).unwrap();
    assert!(calls.contains("KEYCODE_WAKEUP"));
    assert!(calls.contains("wm dismiss-keyguard"));
    assert!(calls.contains(
        "stay_on_while_plugged_in 0; input keyevent KEYCODE_HOME; input keyevent KEYCODE_SLEEP"
    ));
}
