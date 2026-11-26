# Contributing to Hive

These rules apply to everyone, and to any coding agent working on their behalf.
Project layout, coding rules, commands and checks are in [AGENTS.md](AGENTS.md).

## Issues

- One problem or request per issue. Use the issue forms.
- Bugs: what you ran, what happened, and the output of `hive --version` and `hive phones`.
  Steps from a fresh start are the fastest way to a fix.
- **TikTok changed something** (a step says a button "wasn't there" since an app update): use
  the *TikTok changed* form. Attach the screen's XML from `hive screen --xml`, taken on the
  screen where it stopped, after removing anything personal from it.
- Features: state the problem and the acceptance criteria. A feature is ready to build once a
  maintainer labels it `planned`.
- Labels track status: `needs-info` waits on the reporter, `needs-decision` waits on a
  maintainer, `planned` is accepted.

## Pull requests

- One problem per PR. If it resolves an issue, link it with `Closes #123`.
- Reproduce the problem first, or write a test that fails before the fix. Flows are tested
  against a pretend TikTok (`crates/tiktok/tests/fake_app.rs`): teach it the screen you're fixing.
- Fix the cause, and check whether the same cause breaks anything nearby (other phones, other
  Android versions, videos and slideshows alike).
- Keep the diff focused. Unrelated refactors go in their own PR.
- Title: [Conventional Commits](https://www.conventionalcommits.org/), e.g.
  `fix(tiktok): find the Post button after TikTok 48 renamed it`.
- Fill in the template. Keep the description short and in plain words. If it doesn't fit in a
  few sentences, the PR is probably too big.
- **How it was verified** is required: the commands you ran and their results. Anything that
  touches a phone needs a run on a real one: the phone model, Android and TikTok versions, and
  what happened (`--draft` is enough). Screenshots from a phone must not show personal content.
- If a change needs a product decision you can't make, ask in the issue and stop rather than
  guessing.

## Fixing a selector

The most useful contribution. When TikTok renames a button, the fix is usually one line in
[`crates/tiktok/selectors.toml`](crates/tiktok/selectors.toml):

1. `hive screen --xml` on the screen where Hive stopped.
2. Find the element and pick what identifies it: its `text`, `content-desc` or a resource id
   that reads as a word (most of TikTok's ids are obfuscated and change with every version).
3. Try it in `~/.config/hive/tiktok.toml` first (only that key), then run the flow with
   `--draft`.
4. Move the fix into `selectors.toml` and say which TikTok version you read it from.

Keep a selector working for the previous TikTok version when you can: add a candidate to the
list instead of replacing it.
