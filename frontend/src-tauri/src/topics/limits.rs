//! Tuning constants for Topics (specs/0089). Each is defined once, here.

/// Character budget for the transcript evidence handed to the model.
pub const EVIDENCE_CHAR_BUDGET: usize = 24_000;
/// Evidence blocks are labelled in windows of this many seconds.
pub const BLOCK_SECONDS: f64 = 30.0;
/// Most open decisions shown to the revision detector per topic.
pub const OPEN_DECISIONS_CAP: usize = 30;
/// Most bullets and decisions in one meeting's entry. The cap is in the prompt too: without
/// it the built-in 2B model repeats one bullet until it runs out of tokens on a long meeting.
pub const ENTRY_MAX_BULLETS: usize = 8;
pub const ENTRY_MAX_DECISIONS: usize = 5;
/// Most open topics considered by one mention scan.
pub const MENTION_TOPICS_CAP: usize = 20;
/// Most mention suggestions kept per topic per meeting.
pub const MAX_MENTIONS_PER_TOPIC: usize = 3;
/// Longest suggestion text, in characters.
pub const SUGGESTION_TEXT_MAX: usize = 280;
/// How far back "also add this series' meetings" reaches.
pub const BACKFILL_DAYS: i64 = 90;
/// Most meetings sampled when drafting a topic description.
pub const DESCRIBE_MEETINGS_CAP: usize = 5;
