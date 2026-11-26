use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Result, Serial};

/// How to find something on the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selector {
    /// A UiSelector expression: `new UiSelector().text("Post")`.
    UiAutomator(String),
    /// An XPath; under an element, "/*/*" are its children.
    XPath(String),
}

impl Selector {
    pub fn ui(s: impl Into<String>) -> Self {
        Selector::UiAutomator(s.into())
    }
}

/// A UI element's id, valid in the session that found it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElementId(pub String);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }
    pub fn right(&self) -> i32 {
        self.x + self.width
    }
    pub fn area(&self) -> i64 {
        i64::from(self.width) * i64::from(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Android key codes hive uses.
pub mod keycode {
    pub const BACK: u32 = 4;
    pub const HOME: u32 = 3;
    pub const ENTER: u32 = 66;
    pub const ESCAPE: u32 = 111;
}

/// A live UI session on one phone (UiAutomator2). Everything the TikTok flows need.
pub trait Ui {
    /// Every element `selector` finds, under `within` when given.
    fn find_all(
        &mut self,
        selector: &Selector,
        within: Option<&ElementId>,
    ) -> Result<Vec<ElementId>>;
    fn text(&mut self, el: &ElementId) -> Result<String>;
    fn rect(&mut self, el: &ElementId) -> Result<Rect>;
    fn click(&mut self, el: &ElementId) -> Result<()>;
    /// Replace what a text box holds with `text`.
    fn set_text(&mut self, el: &ElementId, text: &str) -> Result<()>;
    fn tap(&mut self, x: i32, y: i32) -> Result<()>;
    /// A slow drag that doesn't fling.
    fn drag(&mut self, from: (i32, i32), to: (i32, i32), speed: u32) -> Result<()>;
    fn swipe(&mut self, area: Rect, direction: Direction) -> Result<()>;
    fn key(&mut self, keycode: u32) -> Result<()>;
    fn screen_size(&mut self) -> Result<(i32, i32)>;
    /// The screen's elements as XML, for debugging.
    fn source(&mut self) -> Result<String>;
    fn is_alive(&mut self) -> bool;
}

/// One phone over adb.
pub trait Device {
    fn serial(&self) -> &Serial;
    /// `adb shell <script>`; stdout and stderr.
    fn shell(&self, script: &str) -> Result<String>;
    fn shell_timeout(&self, script: &str, timeout: Duration) -> Result<String>;
    fn push(&self, local: &Path, remote: &str) -> Result<()>;
}
