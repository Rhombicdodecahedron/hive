---
name: hive
description: Plug phones in. They post.
colors:
  comb: "#0e0b07"
  comb-raised: "#17130c"
  hairline: "#2b2417"
  wax: "#f7efdf"
  wax-muted: "#b3a891"
  wax-faint: "#766d5b"
  honey: "#ffc444"
  ember: "#ff7a3d"
  clover: "#9bd67f"
typography:
  display: "Inter, 800"
  body: "Inter, 400"
  mono: "JetBrains Mono, or the system monospace"
---

# Design System: hive

## Overview

**Creative North Star: "The Hive at Night"**

A shelf of phones on their chargers, screens dark, each one a cell of the hive. When
a post is due, one cell lights up, does its work and goes dark again. Hive is mostly
invisible: what people see is a terminal line, a dashboard glanced at between two
other things, and the phones themselves.

Three surfaces: the terminal output, the dashboard (`hive serve`), and the repository
assets (banner, icon, README images). The terminal stays plain; the dashboard and the
assets carry the brand.

### Named Rules

**The Lit Cell Rule.** Colour means activity or state, never decoration. A phone at
rest is drawn in wax on comb; it only takes colour when it is ready, posting, or
blocked.

**The Say What To Do Rule.** Every state that stops a post says what a person does
about it, in one sentence: "allow USB debugging on the phone's screen", not
"unauthorized".

**The Never Guess Rule.** Hive never shows a post as posted unless TikTok's post
screen went away after Post was tapped. Anything less is "left on the phone to finish".

## Terminal output

- Plain text: no colour, no emoji, no box drawing. It must survive a pipe, a log file
  and a paste into an issue.
- One line per phone in `hive phones`, then indented details. Progress lines during a
  post start with two spaces and say what is happening ("picking the files"). The last
  line is the outcome: `posted`, or `left on the phone for you to finish`.
- Errors go to stderr, prefixed `hive:`, and say what is wrong and what would work.

## Colours

- **Comb** `#0e0b07` is the ground; **Comb Raised** `#17130c` for panels, with a
  **Hairline** `#2b2417` edge.
- **Wax** `#f7efdf` for text, **Wax Muted** and **Wax Faint** for secondary text.
- Three colours carry state and nothing else:
  **Clover** for ready and posted, **Honey** for posting and waiting,
  **Ember** for blocked and failed.
- The wordmark and the primary button are the one gradient: Honey to Ember.
- The hexagon is the only shape motif: the icon, the phones' hub in the banner, a
  faint honeycomb behind the banner's phones. Don't add others.

## Typography

- Display and body: Inter. Serials, times, logs and terminal samples: a monospace face.
- Sentence case everywhere. One eyebrow label in spaced capitals on the banner.

## Do's and Don'ts

### Do:

- **Do** show a phone's real state: its battery, its lock, the account TikTok is on.
- **Do** keep the dashboard to one page with no build step and no external requests.
- **Do** use made-up handles and serials in every image and example
  (`@gta6moments`, `@nightclips`, `RFCT20X4KQB`).

### Don't:

- **Don't** colour terminal output.
- **Don't** show a real person's screen, handle or serial in an asset.
- **Don't** imitate TikTok's own branding: no TikTok logo, no TikTok colours.
- **Don't** use emoji in output, documentation or assets.
