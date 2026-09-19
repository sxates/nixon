// Meeting view-model DTOs + list-row shaping (gist extraction), moved from
// api/api.rs (specs/0042 WS3). Serde shapes are frontend-facing — byte-identical
// to the pre-split wire format.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    /// ISO-8601 (RFC 3339) UTC timestamp the meeting was created.
    /// Used by the dashboard for date grouping and time display.
    pub created_at: String,
    /// ISO-8601 (RFC 3339) UTC timestamp the meeting was last updated.
    pub updated_at: String,
    /// Recording length in seconds (max transcript `audio_end_time`).
    /// `None` when the meeting has no timed transcripts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    /// Short, single-line, ≤140 char glanceable snippet for the meeting:
    /// first non-heading line of the summary, else first transcript line,
    /// else `None`. Markdown/whitespace stripped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    /// How the meeting was created (specs/0015): "recorded" | "notes_only" | "imported".
    /// Lets the list/sidebar branch on meeting type. Serialized as `origin` (camelCase).
    pub origin: String,
    /// Linked EventKit calendar event id (specs/0015); `None` for ad-hoc/notes-only meetings.
    /// Serialized as `calendarEventId` via the struct-level `rename_all = "camelCase"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_event_id: Option<String>,
    /// Bounded attendee preview for the meetings-list row (specs/0038 WS8.a): a few
    /// rostered participants (from `meeting_participants` ⨝ `people`). Owner-INCLUSIVE
    /// and flagged (`isCurrentUser`); the frontend applies the display-only owner filter
    /// (WS8.b). Empty (and omitted from the wire) when the meeting has no roster —
    /// existing consumers that don't read it are unaffected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<AttendeePreview>,
    /// Total roster size (owner-INCLUSIVE, matching the Day Agenda's `attendeeCount`), for
    /// the "+N" overflow beyond the previewed few. The frontend subtracts a previewed owner
    /// before display (WS8.b). `0` when there is no roster.
    #[serde(default)]
    pub attendee_count: i64,
    /// Stable 1-based archival ordinal (specs/0057), serialized as `reelNumber`. The
    /// meeting's position in the non-scheduled set ordered oldest-first; rendered as the
    /// zero-padded "REEL 0412" handle on the reel label and meeting-details identity line.
    pub reel_number: i64,
}

/// One previewed attendee on a meetings-list row (specs/0038 WS8.a). Shaped to match the
/// Day Agenda's `AgendaAttendee` (`{ name, email, isCurrentUser }`) so the frontend can
/// reuse the same `AvatarStack` + owner filter for both surfaces.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendeePreview {
    pub name: String,
    pub email: Option<String>,
    /// True for the device owner ("You"); the frontend hides them from avatars/count.
    pub is_current_user: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteMeetingRequest {
    pub meeting_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetingDetails {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    /// How the meeting was created (specs/0015): "recorded" | "notes_only" | "imported".
    /// Lets the UI branch (e.g. hide the transcript tab / audio controls for notes-only).
    pub origin: String,
    /// Linked EventKit calendar event id (specs/0015); `None` for ad-hoc/notes-only meetings.
    #[serde(rename = "calendarEventId", skip_serializing_if = "Option::is_none")]
    pub calendar_event_id: Option<String>,
    pub transcripts: Vec<MeetingTranscript>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetingTranscript {
    pub id: String,
    pub text: String,
    pub timestamp: String,
    // Recording-relative timestamps for audio-transcript synchronization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_start_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_end_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Stable per-meeting speaker key from diarization (specs/0010); NULL until the
    /// meeting is diarized. P1-C renders labels off this + `speaker_name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// Resolved, renamable display name ("You","Speaker 1",…) joined from `speakers`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_name: Option<String>,
    /// True once the user has manually corrected this segment's text (specs/0061
    /// W5). Always serialized (never `skip_serializing_if`) so the frontend can
    /// rely on its presence to render an "edited" indicator.
    pub user_edited: bool,
}

impl From<crate::database::models::TranscriptWithSpeaker> for MeetingTranscript {
    fn from(t: crate::database::models::TranscriptWithSpeaker) -> Self {
        MeetingTranscript {
            id: t.id,
            text: t.transcript,
            timestamp: t.timestamp,
            audio_start_time: t.audio_start_time,
            audio_end_time: t.audio_end_time,
            duration: t.duration,
            speaker: t.speaker,
            speaker_name: t.speaker_name,
            user_edited: t.user_edited,
        }
    }
}

/// Meeting metadata without transcripts (for pagination)
#[derive(Debug, Serialize, Deserialize)]
pub struct MeetingMetadata {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
    /// How the meeting was created (specs/0015): "recorded" | "notes_only" | "imported".
    pub origin: String,
    /// Linked EventKit calendar event id (specs/0015); `None` for ad-hoc/notes-only meetings.
    #[serde(rename = "calendarEventId", skip_serializing_if = "Option::is_none")]
    pub calendar_event_id: Option<String>,
    /// Recurring-series key (specs/0036): external id / iCalUID. Threaded to the detail
    /// page's record control (specs/0041 WS3) so a recording started there groups into
    /// its series; the frontend DTO already declared it but it was never serialized.
    #[serde(
        rename = "calendarSeriesKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub calendar_series_key: Option<String>,
    /// Stable 1-based archival ordinal (specs/0057) — the same number the meetings list
    /// carries, for the detail page's reel label. `None` for 'scheduled' placeholders,
    /// which sit outside the numbering. (This DTO is NOT `rename_all = "camelCase"`, so
    /// the wire name is spelled out explicitly.)
    #[serde(
        rename = "reelNumber",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reel_number: Option<i64>,
}

/// Paginated transcripts response with total count
#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedTranscriptsResponse {
    pub transcripts: Vec<MeetingTranscript>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveMeetingTitleRequest {
    pub meeting_id: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveMeetingSummaryRequest {
    pub meeting_id: String,
    pub summary: serde_json::Value,
}

const GIST_MAX_CHARS: usize = 140;

/// Builds a short, single-line gist from a `summary_processes.result` JSON blob.
/// The result is `{ "markdown": "...", ... }`; older/plain rows may be a bare
/// JSON string. Returns `None` when nothing usable can be extracted.
pub(crate) fn gist_from_summary(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let markdown = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Object(map)) => map
            .get("markdown")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        Ok(serde_json::Value::String(s)) => Some(s),
        // Not JSON we recognise; treat the blob itself as text.
        _ => Some(raw.to_string()),
    }?;
    gist_from_text(Some(&markdown))
}

/// Reduces free text / markdown to a single glanceable line (≤140 chars):
/// takes the first non-empty, non-heading line and strips common inline
/// markdown markers and surrounding whitespace. Returns `None` if nothing
/// remains.
pub(crate) fn gist_from_text(text: Option<&str>) -> Option<String> {
    let text = text?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;

    let cleaned: String = line
        .chars()
        .filter(|c| !matches!(c, '#' | '*' | '_' | '`' | '>'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }

    let gist: String = cleaned.chars().take(GIST_MAX_CHARS).collect();
    Some(gist)
}

#[cfg(test)]
mod gist_tests {
    use super::{gist_from_summary, gist_from_text, GIST_MAX_CHARS};

    #[test]
    fn skips_headings_and_strips_markdown() {
        let md = "# Meeting Title\n\n## Overview\n**Decision:** ship `v1` now.";
        assert_eq!(
            gist_from_text(Some(md)).as_deref(),
            Some("Decision: ship v1 now.")
        );
    }

    #[test]
    fn truncates_to_max_chars() {
        let long = "a".repeat(500);
        let gist = gist_from_text(Some(&long)).unwrap();
        assert_eq!(gist.chars().count(), GIST_MAX_CHARS);
    }

    #[test]
    fn none_when_only_headings_or_empty() {
        assert_eq!(gist_from_text(Some("# only a heading")), None);
        assert_eq!(gist_from_text(Some("   \n  ")), None);
        assert_eq!(gist_from_text(None), None);
    }

    #[test]
    fn summary_prefers_markdown_field() {
        let raw = "{\"markdown\":\"## Notes\\nWe agreed to refactor.\",\"english_cache\":{}}";
        assert_eq!(
            gist_from_summary(Some(raw)).as_deref(),
            Some("We agreed to refactor.")
        );
    }

    #[test]
    fn summary_falls_back_to_plain_blob() {
        // Bare JSON string and non-JSON blobs are treated as text.
        assert_eq!(
            gist_from_summary(Some("\"just a string summary\"")).as_deref(),
            Some("just a string summary")
        );
        assert_eq!(
            gist_from_summary(Some("plain text summary")).as_deref(),
            Some("plain text summary")
        );
        assert_eq!(gist_from_summary(None), None);
    }
}
