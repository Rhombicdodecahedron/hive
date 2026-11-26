# Hive: contributor guide

Rust CLI and local server that posts to TikTok from real Android phones plugged into the
computer: adb for the phone, Appium's UiAutomator2 server (driven directly, no Appium) for the
screen, and TikTok's app driven step by step like a person would.

## Layout

- `crates/core` (`hive-core`): what a phone, an account and a post are, phone status parsing,
  which phone posts for which account (`route.rs`), and the two ports: `Device` (a shell on one
  phone) and `Ui` (a live UI session). No IO, no adb, no HTTP: everything here is unit-tested.
- `crates/adb` (`hive-adb`): the `adb` program. Phone list and status (one `adb shell` per
  phone, `STATUS_SCRIPT`), waking and opening a phone (`session.rs`), pushing files into the
  gallery and deleting them (`gallery.rs`).
- `crates/uia2` (`hive-uia2`): the UiAutomator2 server on the phone. Installing it (`apks.rs`),
  starting it over `am instrument`, and the HTTP client that implements `Ui` (`server.rs`).
- `crates/tiktok` (`hive-tiktok`): TikTok's flows (`flow.rs`) over `Ui` + `Device`, and
  `selectors.toml`, the **only** place that knows what TikTok calls things on screen.
  `tests/fake_app.rs` is a pretend TikTok that answers the built-in selectors.
- `hive/` (the `hive` binary): the CLI (`main.rs`), one post end to end on the right phone with
  per-phone locks (`engine.rs`), the queue (`queue.rs`), `hive serve`'s API, scheduler and
  dashboard (`serve.rs`, `dashboard.html`), and `hive mcp` (`mcp.rs`).
- `assets/`: README images. Their HTML sources aren't in the repo; regenerate at 2× (2720 px wide).

Crates depend inwards only: `core` ← `adb` ← `uia2`, `core` ← `tiktok`, and the binary on all.
`tiktok` never depends on `adb` or `uia2`: it only sees the ports, so it runs against fakes.

## Rules

- **Safety first, in this order.** Never post on an account other than the one asked for
  (`switch_to` fails rather than guess). Never tap Post while a problem is pending; leave the
  phone on the post screen and say what's left. Never type a lock-screen code. Never delete a
  file on the phone that Hive didn't push (`/sdcard/Movies/hive/`, `/sdcard/Pictures/hive/`).
- Look before you tap: every step finds the element first, waits up to its timeout, and turns
  "not there" into a problem with what to do, not an error halfway through.
- TikTok specifics go in `selectors.toml` (selectors, timeouts, pauses), never in Rust.
  A selector that needs a value takes a `{placeholder}`.
- Waiting is not failing. A phone unplugged, locked with a code or busy is
  `Error::is_waiting()`: the queue retries it without counting a try. Keep that distinction
  when adding errors.
- One `adb shell` round trip per phone for status. Filter on the phone (`| grep`): a full
  `dumpsys window` is megabytes.
- Human pace: taps go through `TikTok::tap`/`tap_at`, which pause at random within
  `[pauses]`. Don't add taps that skip it.
- Errors and messages: plain English, say what happened and what to do
  ("allow USB debugging on the phone's screen"), no stack traces in user output.
- `unsafe` is forbidden (workspace lint). Prefer the standard library; ask before adding a
  dependency with a large tree.
- `dashboard.html` is one file with no build step and no external requests. Colours and type
  follow the README banner: black, red `#ff2d55`, cyan `#22d3ff`.
- Comments say why, not what; keep them sparse and match the surrounding style.

## Issues & PRs

Follow [CONTRIBUTING.md](CONTRIBUTING.md) and `.github/pull_request_template.md`: one problem per
PR (`Closes #<n>` when it resolves an issue), reproduce first, keep the diff focused, Conventional
Commits titles, and a **How it was verified** section with the commands you ran and, for anything
that touches a phone, what you saw on a real one. Unclear product decisions: ask in the issue and
stop.

## Skills

Shared agent skills live in `.agents/skills/` (`.claude/skills/` links to them):
`verify` checks a change on a real phone through `hive serve`;
`selector-fix` repairs a TikTok selector from the phone's screen XML;
`issue-fix` takes an issue from reproduction to one PR.

## Commands

- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all`.
- `cargo run -p hive -- phones` (or any subcommand); `HIVE_HOME=<dir>` keeps config and data in
  one folder, `HIVE_ADB=<path>` picks the adb program (the tests use a fake one).
- `cargo run -p hive -- serve --reload`: the dashboard and API on 127.0.0.1:7878; restarts by
  itself when the binary changes and nothing is running.
- `cargo run -p hive -- screen --xml`: the phone's UI as XML, to read selectors from.
- `cargo run -p hive -- post <file> -a @account --draft`: everything but the Post tap.

## Before opening a PR

Run what CI runs: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. A change to a flow, a selector or anything that touches a phone also needs a run
on a real phone (`.agents/skills/verify`), with `--draft` unless the account is a test account.

## Pitfalls

- CI has no phone. The flows are tested against `tests/fake_app.rs`: when you change a flow,
  teach the pretend app the same screens, so the test covers what a phone does.
- TikTok must be in English: the selectors read its English words.
- One UiAutomator session per phone: an Appium session or another `hive` driving the same
  phone kills Hive's. The per-phone lock (`<data>/locks/<serial>.lock`) prevents the second.
- `adb push` keeps the computer's file date; the gallery sorts by date, so `push_to_gallery`
  touches each file. Keep that, or the picker lists the files in the wrong order.
- Screenshots, screen XML and logs from a real phone hold notifications, handles and
  messages. Crop or redact them before they go into an issue or a PR; never commit them.
- Don't test on an account you can't afford to lose: automation is against TikTok's terms.
