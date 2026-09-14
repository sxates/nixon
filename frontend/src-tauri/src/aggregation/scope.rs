//! Scope selection for cross-meeting aggregation (specs/0035).

use serde::{Deserialize, Serialize};

/// Which meetings an aggregation question may draw from.
///
/// All fields are optional; an empty scope means "all meetings, ranked by
/// relevance to the question". Deserialized from the frontend over IPC
/// (camelCase, matching the codebase's event/command payload convention).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregationScope {
    /// Explicit (pinned) meeting set — nothing outside it is ever selected, so
    /// a run over the ids a preview showed can't diverge from what the user
    /// consented to. The question's FTS ranking still applies WITHIN the set
    /// (order + best-hit metadata, so transcript excerpts keep working); pinned
    /// ids without an FTS hit are appended after, newest first.
    pub meeting_ids: Option<Vec<String>>,
    /// ISO-8601 UTC instant lower bound on `meetings.created_at`, **inclusive**
    /// (compared with SQLite `datetime()`). The frontend sends local-midnight
    /// day boundaries converted to UTC.
    pub date_from: Option<String>,
    /// ISO-8601 UTC instant upper bound on `meetings.created_at`, **exclusive**
    /// — the start (local midnight, in UTC) of the day AFTER the selected end
    /// day.
    pub date_to: Option<String>,
    /// `people.id` — a meeting is in scope when the person was on the roster
    /// (`meeting_participants`, excluding tombstoned/removed rows) OR actually spoke
    /// (`speakers.person_id`); union semantics per the spec.
    pub person_id: Option<String>,
}
