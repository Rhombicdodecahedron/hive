//! hive's core: what a phone, an account and a post are, which phone posts for which
//! account, and the two ports the adapters implement: [`Device`] (adb) and [`Ui`] (a live
//! UiAutomator session). Nothing in here touches a phone.

mod error;
mod handle;
mod memory;
mod phone;
mod post;
mod route;
mod ui;

pub use error::{Error, Result};
pub use handle::Handle;
pub use memory::{PhoneMemory, PhoneNotes};
pub use phone::{
    Blocker, LOW_BATTERY, Lock, Phone, STATUS_SCRIPT, Serial, Status, TIKTOK_PACKAGES, parse_devices,
    parse_lock, parse_status,
};
pub use post::{Media, Post, PostReport, Visibility};
pub use route::{Choice, choose_phones};
pub use ui::{Device, Direction, ElementId, Rect, Selector, Ui, keycode};
