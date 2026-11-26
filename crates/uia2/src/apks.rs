use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use hive_adb::AdbDevice;
use hive_core::{Device, Error, Result};

pub const SERVER_PACKAGE: &str = "io.appium.uiautomator2.server";
pub const TEST_PACKAGE: &str = "io.appium.uiautomator2.server.test";

const RELEASES: &str =
    "https://api.github.com/repos/appium/appium-uiautomator2-server/releases/latest";

fn is_server(name: &str) -> bool {
    name.starts_with("appium-uiautomator2-server-v") && name.ends_with(".apk")
}

fn is_test(name: &str) -> bool {
    name.starts_with("appium-uiautomator2-server-debug-androidTest") && name.ends_with(".apk")
}

/// The server and test APKs, from `cache` or an Appium install on this computer.
pub fn find_apks(cache: &Path) -> Option<(PathBuf, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = vec![cache.to_path_buf()];
    if let Some(h) = &home {
        for base in [
            h.join(".appium/node_modules"),
            h.join(".npm-global/lib/node_modules"),
            h.join(".local/share/pnpm/global/5/node_modules"),
        ] {
            dirs.push(
                base.join(
                    "appium-uiautomator2-driver/node_modules/appium-uiautomator2-server/apks",
                ),
            );
            dirs.push(base.join("appium-uiautomator2-server/apks"));
        }
    }
    dirs.into_iter().find_map(|d| {
        let names: Vec<PathBuf> = fs::read_dir(&d)
            .ok()?
            .filter_map(|e| Some(e.ok()?.path()))
            .collect();
        let pick = |f: fn(&str) -> bool| {
            names
                .iter()
                .find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(f))
                .cloned()
        };
        Some((pick(is_server)?, pick(is_test)?))
    })
}

/// Download the latest server and test APKs from GitHub into `cache`.
fn download(cache: &Path) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(cache)?;
    let release: serde_json::Value = ureq::get(RELEASES)
        .header("User-Agent", "hive")
        .call()
        .and_then(|mut r| r.body_mut().read_json())
        .map_err(|e| {
            Error::Ui(format!(
                "couldn't reach GitHub for the UiAutomator2 server: {e}"
            ))
        })?;
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let mut got = (None, None);
    for a in assets {
        let (Some(name), Some(url)) = (a["name"].as_str(), a["browser_download_url"].as_str())
        else {
            continue;
        };
        if !is_server(name) && !is_test(name) {
            continue;
        }
        let dest = cache.join(name);
        let mut body = Vec::new();
        ureq::get(url)
            .header("User-Agent", "hive")
            .call()
            .map_err(|e| Error::Ui(format!("downloading {name}: {e}")))?
            .body_mut()
            .with_config()
            .limit(100 * 1024 * 1024)
            .reader()
            .read_to_end(&mut body)?;
        fs::write(&dest, body)?;
        if is_server(name) {
            got.0 = Some(dest)
        } else {
            got.1 = Some(dest)
        }
    }
    match got {
        (Some(s), Some(t)) => Ok((s, t)),
        _ => Err(Error::Ui(
            "the UiAutomator2 server's latest release has no APKs".into(),
        )),
    }
}

fn installed(dev: &AdbDevice) -> Result<bool> {
    let out = dev.shell(&format!("pm path {SERVER_PACKAGE}; pm path {TEST_PACKAGE}"))?;
    Ok(out.lines().filter(|l| l.starts_with("package:")).count() >= 2)
}

/// Make sure the UiAutomator2 server is on the phone: kept when already there (Appium may have
/// put it there), else installed from an Appium install on this computer or from GitHub.
/// Returns true when it had to install it.
pub fn ensure_installed(dev: &AdbDevice, cache: &Path) -> Result<bool> {
    if installed(dev)? {
        return Ok(false);
    }
    let (server, test) = match find_apks(cache) {
        Some(found) => found,
        None => download(cache)?,
    };
    dev.install(&server)?;
    dev.install(&test)?;
    Ok(true)
}
