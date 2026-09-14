pub mod action_item;
pub mod ask_ai_history;
pub mod attendee_photos;
pub mod dismissed_calendar_event;
pub mod google_calendar;
/// Series-scoped deletes for the event cache (specs/0054 W5).
pub mod google_calendar_series;
pub mod meeting;
pub mod meeting_brief;
pub mod meeting_note;
pub mod meeting_participant;
pub mod owner_emails;
pub mod people;
pub mod saved_question;
pub mod search;
/// Aggregation-question meeting ranking (specs/0035; split out of `search.rs` per specs/0056 W3).
pub mod search_rank;
pub mod setting;
pub mod speaker;
pub mod summary;
pub mod summary_outline;
pub mod transcript;
pub mod transcript_chunk;
/// Bounded transcript excerpts for Ask AI evidence (split out of `transcript.rs`, specs/0056 W3).
pub mod transcript_excerpt;
pub mod transcript_speaker_overrides;
pub mod voiceprints;
