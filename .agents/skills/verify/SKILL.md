---
name: verify
description: Check a Hive change on a real Android phone through `hive serve`: phone state, account check, a draft post, screenshots as evidence. Use after changing a flow, a selector, adb or the UI server code, or when asked to try something on the phone.
---

# Verify Hive on a real phone

CI has no phone; this is how a change that touches one gets checked. Never publish while
verifying unless the person explicitly says the account is a test account: use `--draft`
(everything filled in, Post not tapped) and say so.

## Run it

- Build and serve the change from this checkout: `cargo run -p hive --release -- serve --reload`,
  in the background. `--reload` restarts it when the binary changes and nothing is running, so
  after each rebuild there's nothing to restart.
- `curl -s 127.0.0.1:7878/api/phones` must list the phone with `"blocker": null`. A blocker says
  what to ask the person (unlock, allow USB debugging…); don't work around it.
- Use `HIVE_HOME=<scratch dir>` when the person's own queue and config must stay untouched.

## Check, in this order

1. **Status**: battery, lock (`secure`), TikTok installed, `ui_server`.
2. **Account**: `POST /api/phones/{serial}/check` → a job; `GET /api/jobs/{id}` until it's done.
   It should name the account TikTok is on, and the phone should be asleep again afterwards.
3. **Draft post** on an account the phone holds:
   `POST /api/posts {"account":"@x","files":["/abs/clip.mp4"],"caption":"test #hive","draft":true}`.
   Follow the job. Expected: `"posted": false`, no problems, the phone left on TikTok's post screen
   with the description typed. Close the draft on the phone yourself or ask the person to.
4. For a slideshow change, repeat with 3 pictures and check their order in the post.
5. For a "wrong account" change, post as an account the phone doesn't hold: the job must end with
   that phone skipped, nothing pushed left in the gallery, the phone asleep.

## Evidence

- `GET /api/phones/{serial}/screen.png` at each step that matters (`before.png`, `after-1.png`…),
  in a scratch folder. Crop or blur notifications, handles and anything personal.
- The job's lines (`GET /api/jobs/{id}`) go into the PR's **How it was verified**, with the
  phone model and the Android and TikTok versions. Never commit screenshots.
