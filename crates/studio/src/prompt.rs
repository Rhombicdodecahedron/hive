//! What the LLM is asked. The rules come from what makes a short clip get watched to the end;
//! `guidelines` in the config adds the account's own (its hashtags, its tone, what worked).

use crate::moment::Moment;

/// What makes a moment worth a clip. Written from the numbers of a real gaming page: clips
/// watched to the end were the ones where something visibly happens and pays off on screen.
pub const RULES: &str = "\
You pick moments from a long video for vertical short clips (TikTok, Reels, Shorts).

What works, in order:
1. Something visibly happens and pays off inside the clip: a fail, a crash, a death, a twist, \
a surprise, a reaction to what is on screen. The viewer must see the result, not hear about it.
2. A comparison with what players already know (the previous game, real life, another game) \
that shows on screen: a habit that no longer works, something bigger, something new.
3. A secret or easter egg only when it is found in the clip. Never a teaser (\"to be \
continued\", \"part 1\" cut before the payoff).

What doesn't work: long explanations or opinions, tutorials, menus, maps, inventories, \
loading screens, night scenes where little can be seen, and anything that needs the rest of \
the video to make sense.

Shape of a clip:
- 15 to 40 seconds in total (the clip's length is the sum of its parts).
- Start on the action or the line that sets it up, never on silence or \"so, um\". The \
first second decides if people stay.
- Up to 3 parts of the video joined, in order, to cut a dull middle. Keep the end: the \
payoff and the reaction to it.
- The hook is the line written on top of the clip: short (under 60 characters), in the \
video's language, in the words of the audience: \"When you…\", \"X players when…\", \
\"What happens when…\". No clickbait the clip doesn't deliver.
- The caption is the post's description: one or two sentences in the video's language, an \
emoji at most two, then 3 to 5 hashtags.";

pub fn pick_system(guidelines: &str) -> String {
    if guidelines.trim().is_empty() {
        RULES.to_owned()
    } else {
        format!(
            "{RULES}\n\nThis account's own rules:\n{}",
            guidelines.trim()
        )
    }
}

pub fn pick_user(title: &str, transcript: &str, count: usize, min: f64, max: f64) -> String {
    format!(
        "Video: {title}\n\
         Propose the {count} best moments, best first, each {min:.0} to {max:.0} seconds. \
         Times are those of the transcript lines (mm:ss.s); a moment ends a little after the \
         last words when the payoff is visual.\n\n\
         Answer with JSON only, no other text:\n\
         {{\"moments\": [{{\"segments\": [[\"mm:ss.s\", \"mm:ss.s\"]], \"hook\": \"…\", \
         \"caption\": \"… #tag #tag\", \"kind\": \"fail|surprise|comparison|secret|funny\", \
         \"why\": \"one sentence\", \"score\": 1-10}}]}}\n\n\
         Transcript:\n{transcript}"
    )
}

pub const REVIEW_SYSTEM: &str = "\
You check a clip before it is made, from frames taken across it (left to right, top to \
bottom, time in the corner). Reject it when the frames are mostly too dark to see, when a \
menu, map, inventory, phone screen or loading screen fills a large part of it, or when \
nothing seems to happen. Otherwise keep it. Answer with JSON only: \
{\"keep\": true|false, \"reason\": \"one short sentence\"}";

pub fn review_user(m: &Moment) -> String {
    format!(
        "The clip's hook: \"{}\". What should happen: {}. Parts: {}.",
        m.hook,
        if m.why.is_empty() {
            "(not said)"
        } else {
            &m.why
        },
        m.segments_text()
    )
}
