---
name: selector-fix
description: Repair a TikTok selector in crates/tiktok/selectors.toml after TikTok changed its app, from the phone's screen XML. Use when a step reports that a TikTok button or box "wasn't there", or for a "TikTok changed" issue.
---

# Fix a TikTok selector

## 1. Find the screen

- Note the step that failed from Hive's output ("TikTok's Next button wasn't there") and the
  matching key in `crates/tiktok/selectors.toml` (here `[picker] next`).
- Get the phone to that screen (a `--draft` post stops on it, or the person opens it), then
  `hive screen --xml -o screen.xml`. From an issue, use the XML attached to it.

## 2. Read what identifies the element

- In the XML, find the element by what a person sees. Its `text`, `content-desc`, `class`,
  `clickable` and `bounds` are what a UiSelector can use.
- Prefer, in this order: a `content-desc` or `text` that names it; a resource id that reads as a
  word (`upload_hot_area`); a structural rule (the biggest list, the button above the "@handle").
  Never an obfuscated id (`id/l3k`): it changes with every TikTok version.
- Keep the old version working: when the selector is a list (`caption`, `title`, `grids`), add a
  candidate instead of replacing one.

## 3. Try it, then make it the default

1. Put only that key in `~/.config/hive/tiktok.toml` (or `$HIVE_HOME/tiktok.toml`) and rerun the
   flow with `--draft` (see the `verify` skill).
2. Move it into `selectors.toml`, with a comment if the rule isn't obvious. Update the line that
   says which TikTok version the file was read off.
3. If `crates/tiktok/tests/fake_app.rs` matches on the old wording, change it to the new one so
   the test still covers the step. `cargo test -p hive-tiktok`.

## 4. The PR

Title `fix(tiktok): …`, the TikTok and Android versions you read it from, and the draft run's
output. No XML or screenshots with personal content in the PR.
