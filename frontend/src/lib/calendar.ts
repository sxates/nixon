/**
 * Calendar integration shared types + helpers (spec 0008, P2 frontend).
 *
 * Thin TS layer over the P2a Rust/EventKit backend. All calls degrade
 * gracefully: the bare `cargo run` dev binary can't prompt for Calendar access
 * (EventKit needs a bundled, code-signed `.app`), so the status commands return
 * `notDetermined`/`denied` and `api_get_upcoming_meetings` returns `[]`. None of
 * these helpers throw on the happy path — they swallow IPC errors and return a
 * safe default so the dashboard never breaks if calendar calls fail.
 *
 * Backend contract (P2a, already built):
 *   - `api_get_calendar_access_status` -> CalendarAccessStatus
 *   - `api_request_calendar_access`    -> boolean (triggers the macOS prompt)
 *   - `api_get_upcoming_meetings`      -> UpcomingMeeting[]  (default 12h)
 */

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { getDayAgenda, type DayAgendaItem } from './day-agenda';
import { getGoogleCalendarStatus } from './googleCalendar';

/** macOS EventKit authorization status, mirrored from the backend. */
export type CalendarAccessStatus =
  | 'authorized'
  | 'denied'
  | 'notDetermined'
  | 'restricted';

/** One upcoming calendar meeting (now-forward, all-day excluded, start-sorted). */
export interface UpcomingMeeting {
  id: string;
  title: string;
  startsAt: string; // ISO-8601
  endsAt: string; // ISO-8601
  calendarName: string;
  location?: string;
  zoomUrl?: string;
  /**
   * Recurring-series key (specs/0036): EventKit `calendarItemExternalIdentifier` /
   * Google iCalUID, series-level for both sources. Threaded into Join & Record so a
   * recording groups into its series. Absent when the event has no external id.
   */
  externalId?: string | null;
}

/** Read the current calendar access status. Never throws. */
export async function getCalendarAccessStatus(): Promise<CalendarAccessStatus> {
  try {
    const status = await invoke<CalendarAccessStatus>('api_get_calendar_access_status');
    return status ?? 'notDetermined';
  } catch (err) {
    // Bare dev binary / command unavailable: behave as "not yet decided".
    console.warn('[calendar] getCalendarAccessStatus failed:', err);
    return 'notDetermined';
  }
}

/**
 * Whether Nixon has a calendar at all (specs/0069 W4).
 *
 * `getCalendarAccessStatus()` is EventKit ONLY, and Today used it alone to decide whether
 * to nag about connecting a calendar — so a user who connected Google and never granted
 * EventKit was told forever to connect the calendar they had already connected. Nixon uses
 * one source at a time; either one counts.
 */
export async function isAnyCalendarConnected(): Promise<boolean> {
  const [eventkit, google] = await Promise.all([
    getCalendarAccessStatus().catch(() => 'denied' as CalendarAccessStatus),
    getGoogleCalendarStatus()
      .then((s) => s.connected)
      .catch(() => false),
  ]);
  return eventkit === 'authorized' || google;
}

/**
 * Trigger the macOS Calendar permission prompt. Returns whether access was
 * granted. Never throws (returns false on any failure).
 */
export async function requestCalendarAccess(): Promise<boolean> {
  try {
    return (await invoke<boolean>('api_request_calendar_access')) === true;
  } catch (err) {
    console.warn('[calendar] requestCalendarAccess failed:', err);
    return false;
  }
}

/**
 * Fetch upcoming meetings within `withinHours` (default 12). Returns `[]` if not
 * authorized or on any failure. Never throws.
 */
export async function getUpcomingMeetings(withinHours = 12): Promise<UpcomingMeeting[]> {
  try {
    const result = await invoke<UpcomingMeeting[]>('api_get_upcoming_meetings', { withinHours });
    return Array.isArray(result) ? result : [];
  } catch (err) {
    console.warn('[calendar] getUpcomingMeetings failed:', err);
    return [];
  }
}

/**
 * One calendar-event occurrence referenced from a meeting row (specs/0041 WS3):
 * the detail page knows the row's `calendar_event_id` + occurrence start, and needs
 * to find the live agenda item (which carries the join link) for that occurrence.
 */
export interface OccurrenceRef {
  /** The meeting row's stored `calendar_event_id`. */
  calendarEventId: string;
  /** The occurrence start (ISO-8601) — the row's `created_at`. */
  occurrenceStart: string;
  /** The meeting row id, matched against agenda items already linked to a recording. */
  meetingId?: string | null;
  /** The row's `calendar_series_key` (external id / iCalUID), the sync-stable fallback. */
  seriesKey?: string | null;
}

/** Tolerance when matching an occurrence to an agenda item by series key + start time. */
const OCCURRENCE_START_TOLERANCE_MS = 30 * 60 * 1000;

/**
 * Find the day-agenda item for one calendar-event occurrence (specs/0041 WS3).
 * Pure matcher over an already-fetched agenda, strongest key first:
 *  1. `meetingId` — the agenda item is already linked to this very meeting row;
 *  2. `calendarEventId` — an unrecorded calendar item's `id` IS the event id;
 *  3. `seriesKey` + start time (±30 min) — survives EventKit reissuing
 *     `eventIdentifier` between reads (the external id is the sync-stable key,
 *     but it's series-level, so the start time pins the occurrence).
 */
export function findAgendaItemForOccurrence(
  items: DayAgendaItem[],
  ref: OccurrenceRef,
): DayAgendaItem | null {
  const calendarItems = items.filter((it) => it.source === 'calendar');

  if (ref.meetingId) {
    const byMeeting = calendarItems.find((it) => it.meetingId === ref.meetingId);
    if (byMeeting) return byMeeting;
  }

  const byEventId = calendarItems.find((it) => it.id === ref.calendarEventId);
  if (byEventId) return byEventId;

  const seriesKey = ref.seriesKey?.trim();
  const startMs = new Date(ref.occurrenceStart).getTime();
  if (seriesKey && Number.isFinite(startMs)) {
    const bySeries = calendarItems.find((it) => {
      if ((it.seriesKey ?? '').trim() !== seriesKey) return false;
      const itemStartMs = new Date(it.startTime).getTime();
      return (
        Number.isFinite(itemStartMs) &&
        Math.abs(itemStartMs - startMs) <= OCCURRENCE_START_TOLERANCE_MS
      );
    });
    if (bySeries) return bySeries;
  }

  return null;
}

/**
 * Resolve the join link (Zoom/Meet/Teams) for one calendar-event occurrence via the
 * same source the Home agenda uses (`api_get_day_agenda`, which extracts `zoomUrl`
 * from the event). Used by the meeting-detail record control (specs/0041 WS3), which
 * only has the meeting row's stored calendar linkage — not a live event object.
 * Returns null when the event has no link or can't be found; never throws.
 */
export async function resolveOccurrenceJoinUrl(ref: OccurrenceRef): Promise<string | null> {
  const start = new Date(ref.occurrenceStart);
  if (Number.isNaN(start.getTime())) return null;
  // Local YYYY-MM-DD, the `api_get_day_agenda` date contract (specs/0038 WS4).
  const date = `${start.getFullYear()}-${String(start.getMonth() + 1).padStart(2, '0')}-${String(
    start.getDate(),
  ).padStart(2, '0')}`;
  const items = await getDayAgenda(date); // never throws; [] on failure
  return findAgendaItemForOccurrence(items, ref)?.zoomUrl ?? null;
}

/**
 * Open a meeting's join URL via the existing Rust `open_external_url` command.
 * Never throws, but a failure is no longer silent (specs/0029 WS1.1): the user
 * gets a toast so "nothing happened" clicks are explained.
 */
export async function openMeetingUrl(url: string): Promise<void> {
  try {
    await invoke('open_external_url', { url });
  } catch (err) {
    console.error('[calendar] openMeetingUrl failed:', err);
    toast.error('Could not open the meeting link', {
      description: 'Try joining from your calendar app instead.',
    });
  }
}

/**
 * Convert an https Zoom join link into a `zoommtg://` desktop-client deep link
 * (spec 0008, P3). Launching the deep link drops the user straight into the Zoom
 * client instead of the browser bounce page.
 *
 * Parses `https://[sub.]zoom.us/j/<confno>[?pwd=<x>]` (the standard join URL) and
 * produces `zoommtg://zoom.us/join?confno=<confno>[&pwd=<x>]`. Only the numeric
 * meeting id from `/j/<id>` and the `pwd` query param are carried over (the hashed
 * `pwd` zoom puts in invite links is the form `zoommtg://` accepts).
 *
 * Returns `null` if the URL isn't a recognizable Zoom join link, so callers can
 * fall back to opening the original https URL.
 */
export function toZoomDeepLink(zoomUrl: string): string | null {
  if (!zoomUrl) return null;
  let parsed: URL;
  try {
    parsed = new URL(zoomUrl);
  } catch {
    return null;
  }
  // Only handle zoom.us (incl. vanity subdomains like company.zoom.us).
  if (!/(^|\.)zoom\.us$/i.test(parsed.hostname)) return null;
  // Standard join path is /j/<confno> (digits). Bail on anything else.
  const match = parsed.pathname.match(/\/j\/(\d+)/);
  if (!match) return null;
  const confno = match[1];

  const params = new URLSearchParams({ confno });
  const pwd = parsed.searchParams.get('pwd');
  if (pwd) params.set('pwd', pwd);

  return `zoommtg://zoom.us/join?${params.toString()}`;
}

/**
 * Open a Zoom meeting, preferring the `zoommtg://` desktop-client deep link and
 * falling back to the original https URL when the link can't be parsed OR the
 * deep-link open fails (specs/0029 WS1.1 — the 0028 allowlist rejected the
 * scheme for a while, and the failure was swallowed, so Zoom silently never
 * launched). Both go through the existing `open_external_url` Rust command —
 * macOS `open` handles custom URL schemes. Never throws; if both attempts fail
 * `openMeetingUrl` surfaces the error toast.
 */
export async function openZoomMeeting(zoomUrl: string): Promise<void> {
  const deepLink = toZoomDeepLink(zoomUrl);
  if (deepLink) {
    try {
      await invoke('open_external_url', { url: deepLink });
      return;
    } catch (err) {
      console.warn('[calendar] zoom deep link failed; falling back to https:', err);
    }
  }
  await openMeetingUrl(zoomUrl);
}

/**
 * Delay between launching Zoom and starting the Nixon recording for one-click
 * "Join & Record". Creating Nixon's Core Audio process tap while Zoom is still
 * initializing its own audio engine makes both processes hit `coreaudiod` at
 * once and reliably hangs the Zoom client (must be force-quit). Letting Zoom
 * fully launch + join first — the same situation as recording an already-running
 * meeting — avoids the collision. Capturing the first few seconds of the call is
 * not important, so the tradeoff is acceptable.
 *
 * Zoom typically launches + joins in ~2-3s; 3s clears that with a small margin.
 * If a cold Zoom launch ever hangs again, bump this to 5s+.
 */
export const JOIN_AND_RECORD_DELAY_MS = 3000;

/**
 * The calendar event a "Join & Record" click is acting on. Carries the bits the
 * recording needs to ADOPT the event's identity (spec 0015, Phase C): its title
 * and its stable EventKit id (`calendarEventId`), plus the join link.
 */
export interface JoinAndRecordEvent {
  /** EventKit event id — stored as `meetings.calendar_event_id` for exact attendee lookup. */
  id: string;
  /** Event title — becomes the meeting title (no date-stamp). */
  title: string;
  /** Optional Zoom/Meet/Teams join link. */
  zoomUrl?: string | null;
  /**
   * Event scheduled start (ISO-8601). The meeting is dated to this (its `created_at`),
   * NOT the moment recording begins — so a calendar meeting always shows the event's
   * time even if you hit record a few minutes late. Optional: omitted → recording time.
   */
  startsAt?: string | null;
  /**
   * Recurring-series key (specs/0036): the event's `external_id` (EventKit
   * `calendarItemExternalIdentifier` / Google iCalUID), series-level for both sources.
   * Persisted as `meetings.calendar_series_key` so the recording groups into its series
   * (and adopts any `scheduled` prep row). Omitted → no series grouping.
   */
  seriesKey?: string | null;
}

/**
 * sessionStorage key carrying a pre-created Join & Record meeting across the
 * navigation to `/record`. The pre-create happens at click time (so the row has
 * the calendar event's title + id), but the recording starts after a delay and a
 * route change that resets `currentMeeting` to the `intro-call` sentinel — so the
 * id can't be held in React state alone. The recorder's start path reads (and
 * clears) this to reuse the row instead of minting a date-stamped one.
 */
const PENDING_JOIN_MEETING_KEY = 'nixon-join-and-record-meeting';

/**
 * A Join & Record meeting awaiting the recorder. Carries the calendar link so the
 * recorder always ends up with a calendar-linked row (specs/0019 WS6.3):
 *   - `id` set      → pre-create succeeded; the recorder ADOPTS that row.
 *   - `id` null     → pre-create failed; the recorder CREATES the row itself, still
 *                     passing `calendarEventId`/`startsAt` so the event identity
 *                     (title + attendee roster) is preserved instead of degrading to
 *                     a bare date-stamped meeting.
 */
export interface PendingJoinMeeting {
  id: string | null;
  title: string;
  /** EventKit event id — keeps the link even when the recorder creates the row. */
  calendarEventId?: string | null;
  /** Event scheduled start (ISO-8601) so a recorder-created row is dated to the event. */
  startsAt?: string | null;
  /** Recurring-series key (specs/0036) so the recorded row inherits `calendar_series_key`. */
  calendarSeriesKey?: string | null;
}

/** Stash the pending Join & Record meeting for the recorder's start path. */
function stashPendingJoinMeeting(meeting: PendingJoinMeeting): void {
  try {
    sessionStorage.setItem(PENDING_JOIN_MEETING_KEY, JSON.stringify(meeting));
  } catch {
    /* sessionStorage unavailable — fall back to a normal recorded meeting */
  }
}

function parsePendingJoinMeeting(raw: string | null): PendingJoinMeeting | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<PendingJoinMeeting>;
    if (parsed && typeof parsed.title === 'string') {
      return {
        id: typeof parsed.id === 'string' ? parsed.id : null,
        title: parsed.title,
        calendarEventId:
          typeof parsed.calendarEventId === 'string' ? parsed.calendarEventId : null,
        startsAt: typeof parsed.startsAt === 'string' ? parsed.startsAt : null,
        calendarSeriesKey:
          typeof parsed.calendarSeriesKey === 'string' ? parsed.calendarSeriesKey : null,
      };
    }
  } catch {
    /* malformed — treat as nothing pending */
  }
  return null;
}

/**
 * Peek at the pending Join & Record meeting WITHOUT clearing it. Used to dedupe a
 * second Join & Record click on the same event during the pre-record delay (specs/0019
 * WS6.3 gap c) so a fast double-tap can't mint two rows.
 */
export function peekPendingJoinMeeting(): PendingJoinMeeting | null {
  try {
    return parsePendingJoinMeeting(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY));
  } catch {
    return null;
  }
}

/**
 * Reconcile the SQLite id from a completed pre-create into a still-armed stash
 * (specs/0029 WS2.1). No-op if the stash was already consumed by a racing start
 * path (that path awaited the create itself via {@link waitForPendingJoinCreate})
 * or replaced by a different event's Join & Record.
 */
function reconcilePendingJoinMeetingId(calendarEventId: string, meetingId: string): void {
  const pending = peekPendingJoinMeeting();
  if (pending && !pending.id && pending.calendarEventId === calendarEventId) {
    stashPendingJoinMeeting({ ...pending, id: meetingId });
  }
}

/**
 * The Join & Record pre-create currently in flight (specs/0029 WS2.1). The stash is
 * written BEFORE the awaited `api_create_meeting`, so a start path that consumes it
 * during the create can see `id: null`; this slot lets it wait briefly for the real
 * meeting id instead of minting a duplicate/date-stamped row. Resolves to null on
 * create failure — the caller then creates the calendar-linked row itself.
 */
let pendingJoinCreateInFlight: Promise<string | null> | null = null;

/**
 * Wait (bounded) for an in-flight Join & Record pre-create to yield its meeting id.
 * Returns null immediately when no create is in flight, and null after `timeoutMs`
 * or on failure — callers must treat null as "create the row yourself".
 */
export async function waitForPendingJoinCreate(timeoutMs = 5000): Promise<string | null> {
  const inFlight = pendingJoinCreateInFlight;
  if (!inFlight) return null;
  try {
    return await Promise.race([
      inFlight,
      new Promise<null>((resolve) => setTimeout(() => resolve(null), timeoutMs)),
    ]);
  } catch {
    return null;
  }
}

/**
 * Read (and clear) any pending Join & Record meeting. The recorder's start path calls
 * this so it reuses/links the calendar meeting instead of creating a second,
 * date-stamped one. Returns null for the normal manual "New recording" path.
 */
export function consumePendingJoinMeeting(): PendingJoinMeeting | null {
  try {
    const parsed = parsePendingJoinMeeting(
      sessionStorage.getItem(PENDING_JOIN_MEETING_KEY),
    );
    sessionStorage.removeItem(PENDING_JOIN_MEETING_KEY);
    return parsed;
  } catch {
    return null;
  }
}

/**
 * Kick the idempotent participant-roster seed for a calendar-linked meeting (specs/0019
 * WS6.3 gap a). `api_get_meeting_participants` seeds `meeting_participants` from the
 * linked EventKit event on first call, so firing it at create time means the *live*
 * recording shows the attendee roster immediately — not only after the user opens the
 * Participants popover. Best-effort; never throws.
 */
export async function seedMeetingParticipants(meetingId: string): Promise<void> {
  try {
    await invoke('api_get_meeting_participants', { meetingId });
  } catch (err) {
    console.warn('[calendar] participant seed failed:', err);
  }
}

/**
 * One-click "Join & Record" (spec 0015, Phase C): the recording IS the calendar
 * event. On click we stash the event identity for the recorder to adopt, open Zoom,
 * create the `meetings` row with the event's title + `calendarEventId` (origin
 * `'recorded'`) and reconcile its id into the stash, then start the Nixon recording
 * a few seconds later so Zoom's audio engine is settled before we create the Core
 * Audio tap (see {@link JOIN_AND_RECORD_DELAY_MS}). If a recording is already in
 * progress the tap is already up, so we just open Zoom and do NOT create a row.
 *
 * `startRecording` is the existing toggle/auto-start trigger (sets the
 * `autoStartRecording` flag + navigates to `/record`); it self-guards and only
 * ever starts, so a delayed call is a no-op if recording began in the meantime.
 * The recorder's `createMeetingForRecording` reads {@link consumePendingJoinMeeting}
 * and reuses the pre-created row, so no duplicate meeting is minted.
 *
 * Robustness (specs/0019 WS6.3 + specs/0029 WS2.1):
 *   - The pending-join is stashed BEFORE Zoom is opened and BEFORE the awaited
 *     create. Opening Zoom trips the backend `zoom-meeting-detected` prompt, and any
 *     start path that wins during the create/delay window (toast Record, tray,
 *     sidebar) consumes this stash — so it must already carry the event identity,
 *     even without a SQLite id yet. The id is reconciled in when the create returns;
 *     a consume that raced the create waits on {@link waitForPendingJoinCreate}.
 *   - Even if the up-front create fails (stash keeps `id: null`), the recorder still
 *     creates a *calendar-linked* row — the live recording keeps the event title +
 *     attendee roster instead of degrading to a bare date-stamped meeting.
 *   - On a successful pre-create we proactively seed the participant roster so the
 *     in-progress recording shows attendees immediately.
 *   - A second Join & Record click on the same event during the pre-record delay is
 *     a no-op (no duplicate row).
 */
export async function joinAndRecord(
  event: JoinAndRecordEvent,
  isRecording: boolean,
  startRecording: () => void,
): Promise<void> {
  const zoomUrl = event.zoomUrl;
  // Already recording: the tap is up; just open Zoom (don't create a stray row).
  if (isRecording) {
    if (zoomUrl) void openZoomMeeting(zoomUrl);
    return;
  }

  // Dedupe a fast double-tap: if this same event is already armed (pending start),
  // just (re)open Zoom — don't mint a second row (specs/0019 WS6.3 gap c).
  const alreadyArmed = peekPendingJoinMeeting();
  if (alreadyArmed?.calendarEventId && alreadyArmed.calendarEventId === event.id) {
    if (zoomUrl) void openZoomMeeting(zoomUrl);
    return;
  }

  const title = event.title?.trim() || 'Untitled meeting';

  // Stash FIRST (specs/0029 WS2.1) — id null until the create below resolves.
  stashPendingJoinMeeting({
    id: null,
    title,
    calendarEventId: event.id,
    startsAt: event.startsAt ?? null,
    calendarSeriesKey: event.seriesKey ?? null,
  });

  if (zoomUrl) void openZoomMeeting(zoomUrl);

  // Create the meeting row up front with the calendar event's identity so the
  // recording adopts its title + event id (not a date-stamp). Published as the
  // in-flight create so a racing consume can await the id instead of duplicating.
  const createPromise = (async (): Promise<string | null> => {
    try {
      const result = await invoke<{ meeting_id: string }>('api_create_meeting', {
        meetingTitle: title,
        origin: 'recorded',
        calendarEventId: event.id,
        // Date the meeting to the event's scheduled start, not the recording start.
        startedAt: event.startsAt ?? null,
        // specs/0036: persist the series key so this recording (and any scheduled prep
        // row it adopts) groups into its recurring series.
        calendarSeriesKey: event.seriesKey ?? null,
      });
      return result?.meeting_id ?? null;
    } catch (err) {
      console.warn(
        '[calendar] joinAndRecord pre-create failed; recorder will create a calendar-linked row:',
        err,
      );
      return null;
    }
  })();
  pendingJoinCreateInFlight = createPromise;
  const createdId = await createPromise;
  if (pendingJoinCreateInFlight === createPromise) {
    pendingJoinCreateInFlight = null;
  }

  if (createdId) {
    // Seed the roster now so the live recording shows attendees without waiting for
    // the Participants popover to be opened (specs/0019 WS6.3 gap a).
    void seedMeetingParticipants(createdId);
    // Fill the id into the stash if it's still armed (no-op if a racing start
    // already consumed it — that path awaited the create itself).
    reconcilePendingJoinMeetingId(event.id, createdId);
  }

  setTimeout(startRecording, JOIN_AND_RECORD_DELAY_MS);
}

// ---------------------------------------------------------------------------
// Time formatting helpers (local time)
// ---------------------------------------------------------------------------

/** "10:00 AM" — local clock time for the meeting start. */
export function formatClockTime(d: Date): string {
  return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit', hour12: true });
}

/**
 * Human relative label for an upcoming start time, e.g. "now", "in 25m",
 * "in 2h 10m". For times already in the past returns "now" (the backend only
 * returns now-forward meetings, but a row can age past start between refreshes).
 */
export function formatRelativeStart(startsAt: Date, now: Date = new Date()): string {
  const diffMs = startsAt.getTime() - now.getTime();
  const diffMin = Math.round(diffMs / 60000);
  if (diffMin <= 0) return 'now';
  if (diffMin < 60) return `in ${diffMin}m`;
  const hours = Math.floor(diffMin / 60);
  const minutes = diffMin % 60;
  return minutes === 0 ? `in ${hours}h` : `in ${hours}h ${minutes}m`;
}
