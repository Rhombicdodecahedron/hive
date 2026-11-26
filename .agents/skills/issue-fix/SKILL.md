---
name: issue-fix
description: Triage and fix a GitHub issue (or a reported bug) end to end: reproduce, find the root cause, check every sibling case, fix the class rather than the instance, verify, and open one PR. Use when given an issue URL or number, or asked to fix a reported bug and open a PR.
---

# Fix an issue: from one case to the whole class

The reported case is one sample of a defect class. A fix that only makes the reported output go
away is not done: the next question is always "does this break for other phones, other Android
versions, slideshows as well as videos, the queue as well as `hive post`?". Answer that before
anyone asks.

## 1. Read and reproduce

- `gh issue view <n> --comments` (repo `Rhombicdodecahedron/hive`). Read the output closely:
  `hive phones`, Android and TikTok versions, the exact line that failed.
- Locate the code path and confirm the claim in code, with `file:line` evidence. Say plainly
  whether it is a bug, TikTok having changed (→ the `selector-fix` skill), a missing feature, or
  the reporter's setup (lock with a code, TikTok not in English, USB debugging off).
- Reproduce: a unit test in `hive-core`, the fake adb in `crates/adb/tests`, the pretend TikTok in
  `crates/tiktok/tests/fake_app.rs`, or a real phone (the `verify` skill). If it needs a phone or a
  TikTok version you don't have, say so up front.

## 2. Name the root cause as a class

One sentence of the form "<mechanism> is missing/wrong, so any <X> hits it". Typical classes here:

- A selector that holds for one TikTok version, language or screen size only.
- A status read that assumes one Android version's `dumpsys` wording (`parse_status`, `parse_lock`).
- A step that taps without looking first, or turns "not there" into an error instead of a problem.
- Something that waits (no phone, locked, busy) treated as a failure, or the other way round.
- Video and slideshow paths diverging (`Media`, the picker tab, the gallery folder).
- Cleanup skipped on an error path: files left in the gallery, a phone left awake, a lock file left.

## 3. Fix the class

- Fix where the mechanism lives, not where the symptom shows. TikTok wording → `selectors.toml`.
- List the sibling cases you checked and how, even the ones that were fine.
- Add the test that would have caught it; teach the pretend TikTok or the fake adb what the real
  one does.

## 4. Verify and open one PR

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; the
  `verify` skill when it touches a phone.
- One PR, `Closes #<n>`, the template filled in. Unclear product decision: comment on the issue,
  label `needs-decision`, and stop.
