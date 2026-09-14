import type { ChannelRun } from '@/lib/live-channel-split';

export interface Message {
  id: string;
  content: string;
  timestamp: string;
}

export interface Transcript {
  id: string;
  text: string;
  timestamp: string; // Wall-clock time (e.g., "14:30:05")
  sequence_id?: number;
  chunk_start_time?: number; // Legacy field
  is_partial?: boolean;
  confidence?: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time?: number; // Seconds from recording start (e.g., 125.3)
  audio_end_time?: number;   // Seconds from recording start (e.g., 128.6)
  duration?: number;          // Segment duration in seconds (e.g., 3.3)
  // Speaker diarization (specs/0010). Backend (api::MeetingTranscript) serializes these
  // as snake_case. NULL/absent until the meeting has been diarized.
  speaker?: string | null;       // Stable per-meeting key ("local","spk_0",…)
  speaker_name?: string | null;  // Resolved display name ("You","Speaker 1",…)
  // Capture-channel tag (specs/0029 WS3.4): 'microphone' | 'system' | 'mixed'.
  // Set at capture time from pre-mix RMS dominance; passed through on save so the
  // offline diarization pass can keep mic-tagged segments attributed to "You".
  channel?: string | null;
  // specs/0055: the same evidence at 600 ms window resolution, tiling this row's
  // span. `channel` above is its lossy whole-row summary. Render-time only — the
  // backend ignores this field on save.
  channel_runs?: ChannelRun[];
}

export interface TranscriptUpdate {
  text: string;
  timestamp: string; // Wall-clock time for reference
  source: string;
  sequence_id: number;
  chunk_start_time: number; // Legacy field
  is_partial: boolean;
  confidence: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time: number; // Seconds from recording start
  audio_end_time: number;   // Seconds from recording start
  duration: number;          // Segment duration in seconds
  // Live speaker diarization (specs/0011, P3-B). Resolved display name
  // ("You" / "Speaker 2") for segments already labeled by the time the event is
  // emitted; null until labeled, and always null when live diarization is off.
  speaker?: string | null;
  // Capture-channel tag (specs/0029 WS3.4): 'microphone' | 'system' | 'mixed', or
  // null when the pipeline had no channel evidence for the segment's span.
  channel?: string | null;
  // specs/0055: window-resolution channel runs tiling this segment's span, so a
  // row straddling an owner<->remote handoff can be split at render time instead
  // of taking one whole-row label. Absent when there was no channel evidence.
  channel_runs?: ChannelRun[];
}

/** Payload of the `live-diarization-update` event (specs/0011, P3-B). Carries
 *  retroactive speaker labels for live transcript segments that were emitted
 *  before a diarization pass resolved them. Each `segment_id` is a transcript
 *  `sequence_id` rendered as a string; correlate by `parseInt(segment_id)`.
 *  `meeting_id` is the in-progress recording's *name* (a scoping label, not a DB
 *  id) — rows are correlated by `segment_id`, not by this. The backend only ever
 *  sends `null → labeled` transitions, so each update is applied as a one-way set. */
export interface LiveDiarizationUpdate {
  meeting_id: string;
  segments: Array<{ segment_id: string; speaker: string }>;
}

export interface Block {
  id: string;
  type: string;
  content: string;
  color: string;
}

export interface Section {
  title: string;
  blocks: Block[];
}

export interface Summary {
  [key: string]: Section;
}

export interface ApiResponse {
  message: string;
  num_chunks: number;
  data: any[];
}

export interface SummaryResponse {
  status: string;
  summary: Summary;
  raw_summary?: string;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}

// BlockNote-specific types
export type SummaryFormat = 'legacy' | 'markdown' | 'blocknote';

export interface BlockNoteBlock {
  id: string;
  type: string;
  props?: Record<string, any>;
  content?: any[];
  children?: BlockNoteBlock[];
}

/**
 * Chunk-outcome accounting attached to a stored summary result as a top-level
 * `summary_status` field (specs/0028; emitted from
 * frontend/src-tauri/src/summary/service.rs). `complete: false` means some
 * transcript chunks failed after retries and the summary is PARTIAL — the UI
 * must warn instead of presenting it as complete.
 */
export interface SummaryChunkStatus {
  complete: boolean;
  total_chunks: number;
  processed_chunks: number;
  failed_chunks: number;
}

export interface SummaryDataResponse {
  markdown?: string;
  summary_json?: BlockNoteBlock[];
  /** Partial-summary indicator (specs/0028) — absent on results saved before it shipped. */
  summary_status?: SummaryChunkStatus;
  // Legacy format fields
  MeetingName?: string;
  _section_order?: string[];
  [key: string]: any; // For legacy section data
}

// Pagination types for optimized transcript loading
export interface MeetingMetadata {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  folder_path?: string;
  // How the meeting was created (spec 0015). 'recorded' (default for existing/recorded
  // meetings) | 'notes_only' | 'imported' | 'scheduled' (pre-call prep placeholder, spec 0036;
  // adopted to 'recorded' at Join & Record). Absent on legacy DTOs ⇒ treat as 'recorded'.
  origin?: 'recorded' | 'notes_only' | 'imported' | 'scheduled';
  calendarEventId?: string | null;
  // Recurring-series key (spec 0036): the calendar event's external_id / iCalUID.
  calendarSeriesKey?: string | null;
  // Archival reel ordinal (spec 0057): 1-based position in the non-scheduled set,
  // oldest first. Absent for 'scheduled' placeholders and on legacy DTOs.
  reelNumber?: number;
}

export interface PaginatedTranscriptsResponse {
  transcripts: Transcript[];
  total_count: number;
  has_more: boolean;
}

// Transcript segment data for virtualized display
export interface TranscriptSegmentData {
  id: string;
  timestamp: number; // audio_start_time in seconds
  endTime?: number; // audio_end_time in seconds
  text: string;
  confidence?: number;
  // Speaker diarization (specs/0010). Absent until the meeting is diarized; when
  // present, `speakerName` is the label shown above the segment text.
  speaker?: string | null;       // stable per-meeting key ("local","spk_0",…)
  speakerName?: string | null;   // resolved display name ("You","Speaker 1",…)
}

/** A diarized speaker for a meeting (specs/0010, P1-C/P2). Mirrors `SpeakerDto`
 *  from `diarization::commands` (camelCase via serde). `email` is populated when
 *  the speaker has been associated with a calendar attendee (P2). */
export interface MeetingSpeaker {
  speakerKey: string;
  displayName: string;
  isLocal: boolean;
  email?: string | null;
  /** Durable Person this speaker is linked to (specs/0016 1b); null until assigned.
   *  Anchor for consolidating speakers mapped to the same person (specs/0019 WS2.4). */
  personId?: string | null;
}

/** A calendar attendee for a meeting (specs/0010 P2 added-scope; sourced from the
 *  linked EventKit event via `api_get_meeting_attendees`). camelCase via serde. */
export interface MeetingAttendee {
  name: string;
  email: string;
  isCurrentUser: boolean;
  /**
   * True when this entry is a distribution list / group address rather than a person
   * (specs/0038 WS3). A DL is NOT speaker-bindable — it's filtered out of the speaker
   * assign pick-lists and surfaced as a labeled row in ParticipantsPanel instead.
   */
  isDistributionList?: boolean;
  /**
   * A self-contained `data:image/...;base64,...` URI for this attendee's directory
   * profile photo, when cached from the Google domain directory (specs/0038 WS3). Render
   * directly in `<img src>` — local-only, no network. Absent → initials fallback.
   */
  photoDataUri?: string;
}

/** A pre-computed obvious 1:1 speaker→attendee mapping, or null when none is clear.
 *  Returned alongside the attendee roster by `api_get_meeting_attendees`. */
export interface AttendeeSuggestion {
  speakerKey: string;
  displayName: string;
  email: string;
}

/** Response shape of `api_get_meeting_attendees` (specs/0010 P2). */
export interface MeetingAttendeesResponse {
  attendees: MeetingAttendee[];
  suggestion: AttendeeSuggestion | null;
}

/** A cross-meeting voice-identity suggestion for a detected speaker (specs/0016 1a).
 *  Returned by `api_get_speaker_suggestions` and carried on the `diarization-complete`
 *  event. Comes from a local cosine match against prior identified speakers — never
 *  auto-applied; the UI surfaces it as a one-click "Looks like …" chip. camelCase via
 *  serde. `suggestedEmail` is present when the matched prior speaker had a calendar
 *  email, enabling the attendee-assign path; otherwise a plain rename applies.
 *  `basis` is human-readable text (e.g. "matched Priya's voice from 2 prior meetings"). */
export interface SpeakerSuggestion {
  speakerKey: string;
  suggestedName: string;
  suggestedEmail: string | null;
  confidence: number;
  basis: string;
}

/** The durable, editable participant roster for a meeting (specs/0017). Distinct from
 *  `MeetingSpeaker` (who actually *spoke*) — a participant is "invited/known". Returned by
 *  `api_get_meeting_participants` / `api_add_meeting_participant`. camelCase via serde.
 *  Each participant IS an app-wide `Person` (`personId` → `people.id`), so role/notes come
 *  for free. `source` records how the row got here: `'calendar'` (seeded from the linked
 *  event) or `'manual'` (hand-added). `email`/`role` are nullable. */
export interface MeetingParticipant {
  personId: string;
  displayName: string;
  email: string | null;
  role: string | null;
  source: 'calendar' | 'manual';
  /** A self-contained base64 `data:` profile photo joined from the cached Google
   *  directory by normalized email (specs/0038 WS3); `null`/absent ⇒ initials. */
  photoDataUri?: string | null;
}

/** Action-item lifecycle (specs/0034). `dismissed` = user rejected an extracted item;
 *  it must stay persisted or the next extraction re-proposes it. */
export type ActionItemStatus = 'open' | 'completed' | 'dismissed';

/** How an action item came to exist (specs/0034): `extracted` by the post-summary LLM
 *  pass, or `manual` (user-added — never touched by re-extraction). */
export type ActionItemSource = 'extracted' | 'manual';

/** A single action item (specs/0034). Mirrors the Rust `ActionItem` DTO (camelCase via
 *  serde). Assignee is exactly one of: a `people` row (`assigneePersonId`), the app owner
 *  (`assigneeIsSelf` — the owner is NOT a people row, roster excludes self), or an
 *  unresolved raw name (`assigneeRaw`, display fallback); all empty = unassigned.
 *  `dueHint` is the verbatim extracted hint ("by Friday") — no date parsing in v1.
 *  `userEdited` marks rows protected from re-extraction. Timestamps are ISO-8601 UTC. */
export interface ActionItem {
  id: string;
  /** Source meeting; null = standalone manual to-do created from the task hub. */
  meetingId: string | null;
  description: string;
  assigneePersonId: string | null;
  assigneeIsSelf: boolean;
  assigneeRaw: string | null;
  /** Verbatim extracted hint ("by Friday") — provenance, never parsed away. */
  dueHint: string | null;
  /** Structured ISO-8601 date (YYYY-MM-DD), sortable/filterable (specs/0038 WS1.a).
   *  Set by the row's date control; lives alongside `dueHint`. null = no date. */
  dueDate: string | null;
  status: ActionItemStatus;
  source: ActionItemSource;
  userEdited: boolean;
  /** Fingerprint of the normalized description (diff identity — backend-owned). */
  contentKey: string;
  createdAt: string;
  updatedAt: string;
  completedAt: string | null;
  /** Manual drag-order key (specs/0038 WS1.b); null = unordered. Only the hub's Manual
   *  sort reads it — the default list order (meeting recency) ignores it. */
  sortOrder: number | null;
}

/** Hub-query row (specs/0034): an action item joined to its source meeting
 *  (`meetings.title`, `meetings.created_at`). Both null for standalone to-dos.
 *  Returned by `api_list_action_items`. */
export interface ActionItemWithMeeting extends ActionItem {
  meetingTitle: string | null;
  meetingCreatedAt: string | null;
}

/** Payload of the `action-items-updated` Tauri event (specs/0034), emitted after a
 *  background extraction commits for a meeting. */
export interface ActionItemsUpdatedPayload {
  meeting_id: string;
}

/** A durable, cross-meeting person (specs/0016 1b). Returned by `api_list_people` /
 *  `api_list_people_ranked` / `api_create_person`. camelCase via serde. `email`/`role`/`notes`
 *  are nullable. `voiceprintOptOut` is the ADR-0007 §2 per-person "don't store this person's
 *  voice" flag — identity association still works when set; it only blocks voice modeling
 *  (wired in Phase 1c). `starred` (specs/0038 WS5.a) pins a person to the top of ranked
 *  pick-lists; toggle it via `api_set_person_starred`. `createdAt`/`updatedAt` are ISO-8601 UTC.
 *  `photoDataUri` (specs/0056 W6) is the cached Google directory photo for the person's email
 *  (a self-contained `data:` URI from `attendee_photos`), absent/null when none is cached —
 *  render initials then. */
export interface Person {
  id: string;
  email: string | null;
  displayName: string;
  role: string | null;
  notes: string | null;
  voiceprintOptOut: boolean;
  starred: boolean;
  createdAt: string;
  updatedAt: string;
  photoDataUri?: string | null;
}

/** A single stored voice sample in a person's gallery (specs/0039 WS3). Returned by
 *  `api_list_person_voiceprints` — metadata/ids only, never the embedding bytes.
 *  `sourceMeetingId` / `sourceSpeakerKey` are the meeting + speaker cluster the sample was
 *  enrolled from (both nullable — legacy samples predate the back-link, so they can only be
 *  quarantined/deleted by id here, never retracted by a span correction). `sampleQuality` is a
 *  0..1 confidence-ish score (nullable). `quarantined` = soft-deleted: excluded from matching
 *  but recoverable via `api_restore_voiceprint_sample`. */
export interface VoiceprintSampleDto {
  id: string;
  sourceMeetingId: string | null;
  sourceSpeakerKey: string | null;
  createdAt: string;
  sampleQuality: number | null;
  quarantined: boolean;
}
