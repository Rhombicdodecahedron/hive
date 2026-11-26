//! The phone's screen, driven through UiAutomator2 without Appium.
//!
//! Appium's own UiAutomator2 server (Apache-2.0) runs on the phone as an instrumentation test
//! and answers HTTP on port 6790; hive forwards a local port to it with adb and talks to it
//! directly. No Node.js, no Appium server: a session opens in about a second, and every phone
//! gets its own server, so several phones post at once.

mod apks;
mod server;

pub use apks::{SERVER_PACKAGE, TEST_PACKAGE, ensure_installed, find_apks};
pub use server::Session;
