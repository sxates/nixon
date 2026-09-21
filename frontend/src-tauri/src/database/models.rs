use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingModel {
    pub id: String,
    pub title: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub folder_path: Option<String>,
    /// How the meeting was created (specs/0015): "recorded" (default) | "notes_only" |
    /// "imported". Column has a NOT NULL DEFAULT 'recorded', so existing/recorded rows
    /// read as "recorded" without a backfill.
    pub origin: String,
    /// EventKit calendar event id this meeting was linked to at Join & Record (specs/0015);
    /// NULL for ad-hoc, notes-only, and pre-existing recorded meetings.
    pub calendar_event_id: Option<String>,
    /// True once the user manually edited the title (specs/0024 WS6.1). Summary auto-titling
    /// must not overwrite it. SQLite stores 0/1; sqlx decodes to bool. Defaults to false.
    #[serde(default)]
    pub title_manually_set: bool,
    /// Per-meeting summary template id (specs/0029 WS4.3 — the specs/0020 persistence
    /// slice); NULL means "use the default template". `#[sqlx(default)]` so legacy
    /// explicit-column SELECTs that predate the column still decode (as None).
    #[serde(default)]
    #[sqlx(default)]
    pub template_id: Option<String>,
    /// Recurring-series key (specs/0036): EventKit `calendarItemExternalIdentifier` /
    /// Google iCalUID — series-level for both calendar sources, so occurrences of the same
    /// recurring meeting share it. NULL for ad-hoc/notes-only/pre-existing rows (they fall
    /// back to normalized-title matching). `#[sqlx(default)]` so explicit-column SELECTs
    /// that predate the column still decode (as None).
    #[serde(default)]
    #[sqlx(default)]
    pub calendar_series_key: Option<String>,
    /// Per-meeting processing-mode override (low-power-mode spec §3):
    /// NULL = follow global, 'live' = force full processing, 'defer' =
    /// force deferral / pending backlog. `#[sqlx(default)]` so legacy
    /// explicit-column SELECTs still decode.
    #[serde(default)]
    #[sqlx(default)]
    pub processing_mode: Option<String>,
    /// Manual-entry occurrence end (specs/0069b review fix 2) — set only for a
    /// Nixon-minted manual scheduled row (`calendar_event_id` prefixed
    /// `nixon-manual:`); NULL everywhere else. `#[sqlx(default)]` so legacy
    /// explicit-column SELECTs that predate the column still decode.
    #[serde(default)]
    #[sqlx(default)]
    pub scheduled_end_at: Option<DateTimeUtc>,
    /// Manual-entry join link (specs/0069b review fix 2) — same scope as
    /// `scheduled_end_at`. `#[sqlx(default)]` so legacy explicit-column SELECTs
    /// still decode.
    #[serde(default)]
    #[sqlx(default)]
    pub join_url: Option<String>,
}

/// Raw row for the enriched meeting-list query (`get_meetings_enriched`).
/// Carries unprocessed inputs (the raw summary JSON blob and the first
/// transcript line); the command layer turns these into the wire `gist`.
#[derive(Debug, Clone, FromRow)]
pub struct MeetingListRow {
    pub id: String,
    pub title: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub duration_seconds: Option<f64>,
    pub summary_result: Option<String>,
    pub first_transcript: Option<String>,
    /// How the meeting was created (specs/0015): "recorded" | "notes_only" | "imported".
    /// Lets the list branch on meeting type (e.g. notes-only meetings hide audio chrome).
    pub origin: String,
    /// Linked EventKit calendar event id (specs/0015); NULL for ad-hoc/notes-only meetings.
    pub calendar_event_id: Option<String>,
    /// Stable 1-based archival ordinal (specs/0057): the meeting's position in the
    /// non-scheduled set ordered by `created_at` ASC (ties broken by `id`). Rendered as
    /// the zero-padded "REEL 0412" handle. Oldest recording is reel 1.
    pub reel_number: i64,
}

/// Raw row for the Day Agenda status query (`get_between_with_status`, specs/0012).
/// One row per meeting created within a local day's window, carrying the per-meeting
/// processing-status flags derived in a single query (no N+1). `title`/timestamps
/// let the merge step build a standalone "recording" agenda item without a second
/// fetch. The boolean flags are SQLite integers (0/1); the command layer maps them
/// to real bools.
#[derive(Debug, Clone, FromRow)]
pub struct MeetingStatusRow {
    pub id: String,
    pub title: String,
    pub created_at: DateTimeUtc,
    pub folder_path: Option<String>,
    /// Recording length in seconds (`MAX(audio_end_time)`), NULL when untimed.
    pub duration_seconds: Option<f64>,
    /// 1 when the meeting has a recording folder set (proxy for "recorded").
    pub has_folder: i64,
    /// 1 when the meeting has ≥1 transcript segment.
    pub has_transcript: i64,
    /// 1 when a summary result is present (`summary_processes.result` non-NULL).
    pub has_summary: i64,
    /// 1 when the meeting has ≥1 diarized speaker row.
    pub has_speakers: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct DateTimeUtc(pub DateTime<Utc>);

/// A manually added meeting's `scheduled`-origin row (specs/0069 W3).
///
/// `calendar_event_id` is the Nixon-minted id (`nixon-manual:{uuid}`), not the meeting's
/// own `id` — later callers (e.g. record-start adoption via `joinAndRecord`) match on
/// `calendarEventId`, so this field must be selected alongside the row's own `id` or a
/// manual entry would mint a second meeting on record instead of adopting this prep row.
#[derive(Debug, Clone, FromRow)]
pub struct ManualScheduledRow {
    pub id: String,
    pub title: String,
    /// The occurrence START, as for every scheduled row.
    pub created_at: DateTimeUtc,
    pub scheduled_end_at: Option<DateTimeUtc>,
    pub join_url: Option<String>,
    pub calendar_event_id: String,
}

impl From<NaiveDateTime> for DateTimeUtc {
    fn from(naive: NaiveDateTime) -> Self {
        DateTimeUtc(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
    }
}

// Renamed from TranscriptSegment to Transcript to match the table name
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub meeting_id: String,
    pub transcript: String,
    pub timestamp: String,
    pub summary: Option<String>,
    pub action_items: Option<String>,
    pub key_points: Option<String>,
    // Recording-relative timestamps for audio-transcript synchronization
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
    /// Per-meeting speaker key written by diarization (specs/0010): "local" for the
    /// mic/local user, "spk_0","spk_1",… for clustered remote speakers. NULL until a
    /// meeting is diarized (today's default), which keeps existing reads unchanged.
    pub speaker: Option<String>,
}

/// A `transcripts` row joined to its `speakers` row so callers get both the stable
/// per-meeting `speaker` key and the resolved, renamable `display_name`. Used by the
/// diarization-aware transcript read; `speaker`/`speaker_name` are NULL for segments
/// of an un-diarized meeting (identical to today's behaviour).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TranscriptWithSpeaker {
    pub id: String,
    pub meeting_id: String,
    pub transcript: String,
    pub timestamp: String,
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
    /// Stable per-meeting speaker key (NULL until diarized).
    pub speaker: Option<String>,
    /// Resolved display name from the `speakers` table ("You","Speaker 1",rename),
    /// NULL when the segment is unlabeled or no matching speaker row exists.
    pub speaker_name: Option<String>,
    /// True once the user has manually corrected this segment's text
    /// (specs/0061 W5, `transcripts::set_segment_text_inner`). Drives the
    /// Enhance dialog's "you have N edited lines" warning before a
    /// regenerate/retranscribe would discard them.
    pub user_edited: bool,
}

/// A diarization speaker for one meeting (specs/0010). Maps a per-meeting
/// `speaker_key` (matches `transcripts.speaker`) to an editable `display_name`.
/// The `embedding` BLOB is reserved for P3 cross-meeting identity and is not
/// selected by P1 reads. Serialized camelCase for the frontend (P1-C).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerModel {
    pub id: String,
    pub meeting_id: String,
    pub speaker_key: String,
    pub display_name: String,
    /// 1 for the mic/local user ("You"), 0 for clustered remote speakers.
    pub is_local: i64,
    /// Calendar-attendee email when the speaker has been associated with a real
    /// person (specs/0010 P2 Task 7); NULL otherwise. Stable cross-meeting identity key.
    pub email: Option<String>,
    /// Durable Person this speaker is linked to (specs/0016 1b), NULL until assigned.
    /// The authoritative anchor for consolidating speakers mapped to the same person
    /// (specs/0019 WS2.4) — more reliable than email, which may be absent.
    pub person_id: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SummaryProcess {
    pub meeting_id: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
    pub result: Option<String>, // JSON
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub chunk_count: i64,
    pub processing_time: f64,
    pub metadata: Option<String>,      // JSON
    pub result_backup: Option<String>, // Backup of result before regeneration
    pub result_backup_timestamp: Option<chrono::DateTime<chrono::Utc>>, // When backup was created
    /// specs/0041 WS2: 1 when the summary was generated from a speaker-attributed
    /// transcript (`any_speaker` in the summary path). 0 → eligible for the one-shot
    /// post-diarization auto-regenerate (summary/refresh.rs).
    pub speaker_attributed: i64,
    /// specs/0041 WS2: fingerprint of the stored `$.markdown` at generation time.
    /// The pristine guard skips the auto-regenerate when the current markdown no
    /// longer matches (user edited). NULL on legacy rows → treated as edited.
    pub generated_markdown_hash: Option<String>,
    /// specs/0044 WS3: fingerprint of the sorted distinct resolved speaker display
    /// names at generation time. The post-naming trigger regenerates a pristine
    /// summary when the current name set no longer matches. NULL on legacy rows →
    /// treated as "names changed" (the markdown pristine guard still applies).
    pub speaker_names_hash: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TranscriptChunk {
    pub meeting_id: String,
    pub meeting_name: Option<String>,
    pub transcript_text: String,
    pub model: String,
    pub model_name: String,
    pub chunk_size: Option<i64>,
    pub overlap: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Setting {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[sqlx(rename = "whisperModel")]
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[sqlx(rename = "groqApiKey")]
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: Option<String>,
    #[sqlx(rename = "openaiApiKey")]
    #[serde(rename = "openaiApiKey")]
    pub openai_api_key: Option<String>,
    #[sqlx(rename = "anthropicApiKey")]
    #[serde(rename = "anthropicApiKey")]
    pub anthropic_api_key: Option<String>,
    #[sqlx(rename = "ollamaApiKey")]
    #[serde(rename = "ollamaApiKey")]
    pub ollama_api_key: Option<String>,
    #[sqlx(rename = "openRouterApiKey")]
    #[serde(rename = "openRouterApiKey")]
    pub open_router_api_key: Option<String>,
    #[sqlx(rename = "ollamaEndpoint")]
    #[serde(rename = "ollamaEndpoint")]
    pub ollama_endpoint: Option<String>,
    /// Custom OpenAI-compatible endpoint configuration stored as JSON
    #[sqlx(rename = "customOpenAIConfig")]
    #[serde(rename = "customOpenAIConfig")]
    pub custom_openai_config: Option<String>,
}

impl Setting {
    /// Parse the custom OpenAI config from JSON string
    pub fn get_custom_openai_config(&self) -> Option<crate::summary::CustomOpenAIConfig> {
        self.custom_openai_config
            .as_ref()
            .and_then(|json| serde_json::from_str(json).ok())
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TranscriptSetting {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[sqlx(rename = "whisperApiKey")]
    #[serde(rename = "whisperApiKey")]
    pub whisper_api_key: Option<String>,
    #[sqlx(rename = "deepgramApiKey")]
    #[serde(rename = "deepgramApiKey")]
    pub deepgram_api_key: Option<String>,
    #[sqlx(rename = "elevenLabsApiKey")]
    #[serde(rename = "elevenLabsApiKey")]
    pub eleven_labs_api_key: Option<String>,
    #[sqlx(rename = "groqApiKey")]
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: Option<String>,
    #[sqlx(rename = "openaiApiKey")]
    #[serde(rename = "openaiApiKey")]
    pub openai_api_key: Option<String>,
}

/// A meeting's notes: the user's own notes (notes_markdown/notes_json) plus, optionally,
/// the AI-enhanced version (enhanced_*). The raw notes are never overwritten by enhancement.
/// See spec 0003 (note-enhancement pipeline). Column names are snake_case in SQLite, so
/// sqlx maps by field name; serde serializes to camelCase for the frontend.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingNote {
    pub meeting_id: String,
    pub notes_markdown: Option<String>,
    pub notes_json: Option<String>,
    pub enhanced_markdown: Option<String>,
    pub enhanced_json: Option<String>,
    pub enhanced_at: Option<String>,
    pub enhanced_model: Option<String>,
    /// Pre-call prep notes (specs/0036): the agenda / "things I plan to cover", authored before
    /// the meeting. Kept separate from `notes_markdown` (live notes). `#[sqlx(default)]` so
    /// explicit-column SELECTs that predate the columns still decode (as None).
    #[serde(default)]
    #[sqlx(default)]
    pub prep_markdown: Option<String>,
    #[serde(default)]
    #[sqlx(default)]
    pub prep_json: Option<String>,
    #[serde(default)]
    #[sqlx(default)]
    pub prep_updated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
