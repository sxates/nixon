/**
 * Day Agenda shared types + helpers (spec 0003-adjacent, replaces the ephemeral
 * "upcoming meetings" list with a persistent whole-day agenda).
 *
 * Thin TS layer over the already-built `api_get_day_agenda` Rust backend. Like
 * `lib/calendar.ts`, every call degrades gracefully: the bare `cargo run` dev
 * binary can't read EventKit, so the backend returns recordings only (or `[]`).
 * `getDayAgenda` never throws — it swallows IPC errors and returns `[]` so the
 * dashboard never breaks.
 *
 * Backend contract (already built, camelCase):
 *   - `api_get_day_agenda` -> DayAgendaItem[]  (today, time-ordered; calendar
 *      events + ad-hoc recordings; best-effort, recordings even if calendar denied)
 */

import { invoke } from '@tauri-apps/api/core';

/** One attendee on a calendar event. */
export interface AgendaAttendee {
  name: string;
  email: string | null;
  isCurrentUser: boolean;
  /**
   * True for a distribution-list / group address (e.g. `team@company.com`) that the
   * backend couldn't expand to individuals (specs/0038 WS3). A DL is not a person: it's
   * excluded from speaker binding and rendered as a labeled floor, never a face.
   * Absent on roster-sourced preview rows (they exclude DLs at seed time).
   */
  isDistributionList?: boolean;
  /**
   * A self-contained `data:image/...;base64,...` URI for this attendee's directory
   * profile photo, when one was cached from the Google domain directory (specs/0038 WS3).
   * Render it directly in `<img src>` — it's LOCAL-ONLY, no network. Absent for external
   * attendees, orgs that forbid the People directory, and every EventKit-sourced attendee;
   * the UI falls back to initials.
   */
  photoDataUri?: string | null;
}

/** Per-item processing status from the backend. */
export interface AgendaStatus {
  recorded: boolean;
  transcribed: boolean;
  summarized: boolean;
  speakersIdentified: boolean;
}

/** One row in the whole-day agenda (calendar event or ad-hoc recording). */
export interface DayAgendaItem {
  id: string;
  title: string;
  startTime: string; // ISO-8601
  endTime: string | null; // ISO-8601
  source: 'calendar' | 'recording' | 'manual';
  zoomUrl: string | null;
  /** First ~5 attendees; full count in `attendeeCount`. */
  attendees: AgendaAttendee[];
  attendeeCount: number;
  /** Recorded meeting row id if recorded/linked, else null. */
  meetingId: string | null;
  status: AgendaStatus;
  /** User hid this calendar event (specs/0026). Filtered out by default; revealable. */
  dismissed: boolean;
  /**
   * Key to WRITE when hiding/unhiding this item (specs/0029 WS6.2): the backend's
   * sync-stable key (EventKit external identifier + occurrence start) when available,
   * else the row `id`. Optional only because a stale same-session cache entry may
   * predate the field — fall back to `id` (see `dismissKeysFor`).
   */
  dismissKey?: string;
  /**
   * Recurring-series key (specs/0036): the calendar event's `external_id`
   * (EventKit `calendarItemExternalIdentifier` / Google iCalUID), series-level for
   * both sources. Threaded into `api_ensure_scheduled_meeting` and Join & Record so a
   * recording groups into its series. `null`/absent for ad-hoc recordings (no event),
   * or when a stale same-session cache entry predates the field.
   */
  seriesKey?: string | null;
  /**
   * The manual row's Nixon-minted event id (`nixon-manual:{uuid}`), specs/0069 W3.
   * Set only for `source: 'manual'` items; `null`/absent for calendar events and
   * ad-hoc recordings.
   */
  calendarEventId?: string | null;
}

/**
 * Fetch a whole-day agenda. `date` is a local `YYYY-MM-DD` (specs/0038 WS4); omit
 * it (or pass `undefined`) for today — the backend treats `None` as today, so the
 * existing call sites keep working unchanged. Returns `[]` on any failure (e.g. bare
 * dev binary, command unavailable, or a malformed date the backend rejects). Never
 * throws — the dashboard degrades to an empty day rather than breaking.
 */
export async function getDayAgenda(date?: string): Promise<DayAgendaItem[]> {
  try {
    // Only thread `date` when provided so the today call is byte-for-byte the old one.
    const result = await invoke<DayAgendaItem[]>(
      'api_get_day_agenda',
      date ? { date } : undefined,
    );
    return Array.isArray(result) ? result : [];
  } catch (err) {
    console.warn('[day-agenda] getDayAgenda failed:', err);
    return [];
  }
}

/**
 * Hide a calendar event from the agenda (specs/0026). `eventKey` is the item's dismissal
 * write key (see `dismissKeysFor`). Throws on failure so callers can surface a toast.
 */
export async function dismissCalendarEvent(eventKey: string): Promise<void> {
  await invoke('api_dismiss_calendar_event', { eventId: eventKey });
}

/**
 * Un-hide a previously dismissed calendar event (specs/0026). Accepts one or more keys
 * (specs/0029 WS6.2: a dismissal may be stored under the item's legacy id OR its new
 * stable key, so unhide clears both — deletes are idempotent). Throws on failure.
 */
export async function undismissCalendarEvent(eventKeys: string | string[]): Promise<void> {
  const keys = Array.isArray(eventKeys) ? eventKeys : [eventKeys];
  await Promise.all(keys.map((eventId) => invoke('api_undismiss_calendar_event', { eventId })));
}

/**
 * Dismissal keys for an agenda item (specs/0029 WS6.2), preferred write key FIRST:
 * the backend's sync-stable `dismissKey` (falling back to `id` for stale cached
 * items that predate the field), then the legacy row `id`. Deduped — callers hide
 * with `[0]` and unhide with the full list so legacy-stored dismissals clear too.
 */
export function dismissKeysFor(item: Pick<DayAgendaItem, 'id' | 'dismissKey'>): string[] {
  return [...new Set([item.dismissKey || item.id, item.id])];
}

/**
 * Pure optimistic-update step for hide/unhide (specs/0029 WS6.2): a copy of `items`
 * with the matching item's `dismissed` flag set. No-op (same contents) when `id`
 * isn't present, so a revert after an error can never invent rows.
 */
export function setItemDismissed(
  items: DayAgendaItem[],
  id: string,
  dismissed: boolean,
): DayAgendaItem[] {
  return items.map((it) => (it.id === id ? { ...it, dismissed } : it));
}

// ---------------------------------------------------------------------------
// Last-good agenda cache (specs/0024 WS5.1)
//
// EventKit cold reads can transiently return success-with-no-calendar-items, and
// `getDayAgenda` returns [] on an outright IPC failure. Both make the home agenda
// flicker (events disappear then reappear). `DayAgenda` already keeps prior calendar
// rows across a transient loss, but that protection didn't cover the very FIRST load
// (no prior state) or the separate decorative count loader on the home page. A small
// per-session last-good cache closes both gaps uniformly. We cache only reads that
// include calendar items — recordings always come back reliably, so a recordings-only
// read must never overwrite a good calendar cache. sessionStorage (not localStorage)
// so it's scoped to the app session and can't outlive a genuine same-day change.
// ---------------------------------------------------------------------------

const AGENDA_CACHE_KEY = 'nixon-day-agenda-cache';

/** Read the last-good cached agenda for this session. Empty array if none/unavailable. */
export function readCachedAgenda(): DayAgendaItem[] {
  try {
    const raw = sessionStorage.getItem(AGENDA_CACHE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as DayAgendaItem[]) : [];
  } catch {
    return [];
  }
}

/**
 * Cache the agenda as last-good. No-op unless the read includes calendar items, so a
 * transient calendar-less read never clobbers a good cache. Best-effort (swallows
 * storage errors).
 */
export function cacheAgenda(items: DayAgendaItem[]): void {
  try {
    if (!items.some((it) => it.source === 'calendar')) return;
    sessionStorage.setItem(AGENDA_CACHE_KEY, JSON.stringify(items));
  } catch {
    /* best-effort cache; ignore quota/serialization errors */
  }
}

// ---------------------------------------------------------------------------
// Time helpers (local time)
// ---------------------------------------------------------------------------

export type AgendaPhase = 'past' | 'now' | 'upcoming';

/**
 * How early — before an event's official start — it flips from "upcoming" to "now",
 * so Join & Record is ready a few minutes ahead of the scheduled time.
 */
export const PRE_START_GRACE_MS = 5 * 60 * 1000;

/**
 * Classify an item by its time window relative to `now`:
 *  - "now"      → within PRE_START_GRACE_MS of the start, started and not yet ended
 *                 (or no end time but started < 90 min ago)
 *  - "past"     → already ended
 *  - "upcoming" → still more than PRE_START_GRACE_MS before the start
 */
export function agendaPhase(item: DayAgendaItem, now: Date = new Date()): AgendaPhase {
  const start = new Date(item.startTime);
  if (Number.isNaN(start.getTime())) return 'upcoming';
  const end = item.endTime ? new Date(item.endTime) : null;
  const nowMs = now.getTime();

  // "now" opens a few minutes before the official start (pre-start grace) so the
  // join action is available slightly early.
  if (start.getTime() - PRE_START_GRACE_MS > nowMs) return 'upcoming';

  if (end && !Number.isNaN(end.getTime())) {
    return end.getTime() >= nowMs ? 'now' : 'past';
  }
  // No end time: treat as "now" for a 90-minute grace window after start.
  const graceMs = 90 * 60 * 1000;
  return nowMs - start.getTime() <= graceMs ? 'now' : 'past';
}

/**
 * Whether an agenda row should offer "Join & Record" (specs/0019 WS6.4).
 *
 * False while ANY recording is in progress: you can't start a second recording, and a
 * *different* happening-now event must not still tempt you to "Join & Record" while
 * you're already recording something else. Also false once the row is recorded or when
 * it has no join link, or when this very row is the one being recorded.
 */
export function canJoinAgendaItem(args: {
  hasZoomUrl: boolean;
  recorded: boolean;
  isRecordingThis: boolean;
  anyRecordingInProgress: boolean;
}): boolean {
  const { hasZoomUrl, recorded, isRecordingThis, anyRecordingInProgress } = args;
  return hasZoomUrl && !recorded && !isRecordingThis && !anyRecordingInProgress;
}

/** Up to two initials for an attendee/avatar (e.g. "Ada Example" → "BS"). */
export function initials(nameOrEmail: string): string {
  const trimmed = nameOrEmail.trim();
  if (!trimmed) return '?';
  // If it's an email, use the local part.
  const base = trimmed.includes('@') ? trimmed.split('@')[0] : trimmed;
  const parts = base.split(/[\s._-]+/).filter(Boolean);
  if (parts.length === 0) return base.slice(0, 1).toUpperCase();
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/**
 * Best display label for an attendee: the human name when present, else the local
 * part of the email (e.g. "jordan.lee@example.com" → "jordan.lee").
 *
 * EventKit (and our backend) fall back to the raw email when a participant has no
 * display name, surfacing it as `name`. So a `name` that is itself an email address
 * is treated the same as an email-only attendee — we never render a raw "...@..."
 * address in the agenda.
 */
export function attendeeLabel(a: AgendaAttendee): string {
  const name = a.name?.trim();
  if (name && !name.includes('@')) return name;
  const email = a.email?.trim() || (name?.includes('@') ? name : null);
  if (email) return email.split('@')[0];
  return 'Guest';
}

// ---------------------------------------------------------------------------
// Manual meetings (specs/0069 W3) — a meeting added inside Nixon, no calendar
// event behind it.
// ---------------------------------------------------------------------------

/** specs/0069 W3 — a meeting added inside Nixon (no calendar event behind it). */
export interface ManualMeetingInput {
  title: string;
  startsAt: string;
  endsAt: string | null;
  joinUrl: string | null;
}

export async function createManualMeeting(input: ManualMeetingInput): Promise<string> {
  return invoke<string>('api_create_manual_meeting', {
    title: input.title,
    startsAt: input.startsAt,
    endsAt: input.endsAt,
    joinUrl: input.joinUrl,
  });
}

export async function updateManualMeeting(
  meetingId: string,
  input: ManualMeetingInput,
): Promise<void> {
  await invoke('api_update_manual_meeting', {
    meetingId,
    title: input.title,
    startsAt: input.startsAt,
    endsAt: input.endsAt,
    joinUrl: input.joinUrl,
  });
}

export async function deleteManualMeeting(meetingId: string): Promise<void> {
  await invoke('api_delete_manual_meeting', { meetingId });
}
