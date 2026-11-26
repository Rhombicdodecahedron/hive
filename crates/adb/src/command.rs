use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use hive_core::{Device, Error, Result, Serial};

pub(crate) const QUICK: Duration = Duration::from_secs(15);
pub(crate) const SLOW: Duration = Duration::from_secs(300);

/// Run adb and return stdout + stderr, killing it after `timeout`.
pub(crate) fn run(exe: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NoAdb
            } else {
                Error::Io(e)
            }
        })?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Adb(format!(
                "`adb {}` took longer than {}s",
                args.join(" "),
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(15));
    }
    let mut text = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(
        &err_reader.join().unwrap_or_default(),
    ));
    Ok(text)
}

/// One phone, through adb.
#[derive(Debug, Clone)]
pub struct AdbDevice {
    exe: PathBuf,
    serial: Serial,
}

impl AdbDevice {
    pub fn new(exe: PathBuf, serial: Serial) -> Self {
        Self { exe, serial }
    }

    fn adb(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let mut all = vec!["-s", self.serial.as_str()];
        all.extend_from_slice(args);
        let out = run(&self.exe, &all, timeout)?;
        if out.contains("device '") && out.contains("not found")
            || out.starts_with("error: device offline")
            || out.contains("error: no devices")
        {
            return Err(Error::PhoneGone(self.serial.clone()));
        }
        Ok(out)
    }

    /// The screen as a PNG.
    pub fn screencap(&self) -> Result<Vec<u8>> {
        let mut child = Command::new(&self.exe)
            .args(["-s", self.serial.as_str(), "exec-out", "screencap", "-p"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut data = Vec::new();
        child.stdout.take().expect("piped").read_to_end(&mut data)?;
        child.wait()?;
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
        // a phone with two screens warns before the picture
        match data.windows(PNG.len()).position(|w| w == PNG) {
            Some(start) => Ok(data.split_off(start)),
            None => Err(Error::Adb(format!(
                "no picture of {}'s screen",
                self.serial
            ))),
        }
    }

    /// Forward a free local port to `remote_port` on the phone; returns the local port.
    pub fn forward(&self, remote_port: u16) -> Result<u16> {
        let out = self.adb(&["forward", "tcp:0", &format!("tcp:{remote_port}")], QUICK)?;
        out.trim()
            .parse()
            .map_err(|_| Error::Adb(format!("adb forward said {:?}", out.trim())))
    }

    pub fn unforward(&self, local_port: u16) {
        let _ = self.adb(
            &["forward", "--remove", &format!("tcp:{local_port}")],
            QUICK,
        );
    }

    pub fn install(&self, apk: &Path) -> Result<()> {
        let apk = apk.to_string_lossy();
        let out = self.adb(&["install", "-r", "-g", &apk], SLOW)?;
        if out.contains("Success") {
            Ok(())
        } else {
            Err(Error::Adb(format!(
                "installing {apk} failed: {}",
                out.trim()
            )))
        }
    }

    /// Start `adb shell <script>` and leave it running (the UI server's instrumentation).
    pub fn spawn_shell(&self, script: &str) -> Result<Child> {
        Ok(Command::new(&self.exe)
            .args(["-s", self.serial.as_str(), "shell", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?)
    }

    pub fn key(&self, keycode: &str) -> Result<()> {
        self.shell(&format!("input keyevent {keycode}")).map(drop)
    }
}

impl Device for AdbDevice {
    fn serial(&self) -> &Serial {
        &self.serial
    }

    fn shell(&self, script: &str) -> Result<String> {
        self.shell_timeout(script, QUICK)
    }

    fn shell_timeout(&self, script: &str, timeout: Duration) -> Result<String> {
        self.adb(&["shell", script], timeout)
    }

    fn push(&self, local: &Path, remote: &str) -> Result<()> {
        let src = local.to_string_lossy();
        let out = self.adb(&["push", &src, remote], SLOW)?;
        if out.contains("error") || out.contains("failed") {
            return Err(Error::Adb(format!("adb push {src}: {}", out.trim())));
        }
        Ok(())
    }
}
