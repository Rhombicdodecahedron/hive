# Product

## Platform

Command-line program for Linux and macOS, written in Rust, with a local server
(`hive serve`: queue runner, JSON API, dashboard) and an MCP server (`hive mcp`). It
drives Android phones plugged into the same computer, over adb. No account, no cloud
service, no TikTok API key. Scope: this record covers the `hive` binary and its crates.
An optional part, the studio (`--features studio`), makes the clips too: a long video in,
moments picked by an LLM the user chooses (a local one or an API), vertical clips out.

## Users

- **Primary: people who run faceless or clipping pages.** Several TikTok accounts, a
  steady flow of clips, a drawer of old Android phones. Posting by hand several times a
  day is the job they want to stop doing, without handing their accounts to a cloud
  scheduler.
- **Secondary: tool builders.** They already make the clips (a pipeline, a script, an
  assistant over MCP) and need the last step: get the file onto TikTok, on the right
  account, at the right time.

## Product Purpose

Make "plug a phone in" the whole setup. Hive picks the phone that holds the account,
wakes it, opens TikTok on that account, picks the file, types the description, chooses
who can see it, taps Post and puts the phone back to sleep. Success means a person
plans a week of posts and comes back to find them posted, or to a clear sentence
saying which phone needs what.

## Positioning

Cloud schedulers post through TikTok's API or a browser session, with the limits and
the account access that come with them. Appium-based scripts drive a phone but need
Node.js, a server and code per app screen. Hive drives the real app on real phones,
like a person, with a single binary, several phones at once, and TikTok's wording kept
in one editable file.

## Operating Context

- Phones sit on chargers, screens off, often for days. A post wakes one up for a
  minute or two.
- TikTok changes its app every few weeks; a renamed button is the usual breakage.
  The fix is a selector in `selectors.toml`, overridable without a release.
- The app must be in English on the phone. The screen lock must be None or Swipe for
  Hive to post unattended.

## Capabilities and Constraints

Hard constraints future work must preserve:

1. Never post on an account other than the one asked for. When the phone's TikTok
   doesn't hold it, Hive moves to another phone or stops.
2. Never tap Post with a problem pending, and never report a post as posted unless
   TikTok's post screen went away after the tap.
3. Never type a lock-screen code, a password or any credential.
4. Never touch files on the phone that Hive didn't push there.
5. Waiting is not failing: a phone unplugged, locked or busy delays a post, it doesn't
   count as a failed try.
6. A post can be stopped until Post is tapped; after that, it is out.
7. `hive serve` listens on 127.0.0.1 by default.

Other facts:

- Ports and adapters: `hive-core` holds the domain with no IO; adb, the UI server and
  TikTok's flows are adapters behind the `Device` and `Ui` ports.
- One post per phone at a time (a lock file per phone); different phones post at once.
- Pauses between taps are random within `[pauses]`: posting at a human pace.

## Brand Commitments

- Name: Hive, capitalised in running text, `hive` for the command.
- Not affiliated with TikTok or ByteDance. TikTok is a trademark of ByteDance. Not
  affiliated with Appium either; Hive uses Appium's UiAutomator2 server under its
  Apache-2.0 licence.
- Automating an account goes against TikTok's terms: the README says so plainly, and
  Hive never claims to be undetectable.
- Voice is plain, short and practical. Messages say what to do.

## Evidence on Hand

- One real run by the author on a Galaxy S21 (Android 15): reading the
  account in 7 s from a sleeping phone, a draft video post filled in up to Post in 28 s.
- The flows are covered by tests against a pretend TikTok; CI has no phone.
- No user counts, ban statistics or testimonials exist. Do not fabricate them.
