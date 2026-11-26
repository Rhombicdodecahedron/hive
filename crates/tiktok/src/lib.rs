//! TikTok's Android app, driven like a person would: open it on the right account, pick the
//! video or the pictures, type the description, choose who can see it, tap Post. What TikTok
//! calls things on screen lives in `selectors.toml`, so a TikTok update means editing a file,
//! not the code.

mod app;
mod flow;

pub use app::App;
pub use flow::TikTok;
