/**
 * Today view timeline (specs/0036 WS7) — pure layout + click-routing helpers.
 *
 * The Home screen is an 8am–6pm vertical hour grid of today's calendar events and
 * recordings (`DayAgendaItem[]` from `api_get_day_agenda`). All the math (grid bounds,
 * per-item pixel offset/height, overlap lanes, the "now" line) and the click-routing
 * decision live here as pure functions so they're unit-testable without a DOM. The
 * component (`app/page.tsx`) owns rendering + the `invoke`/`router` side effects.
 */

import {
  agendaPhase,
  canJoinAgendaItem,
  type DayAgendaItem,
} from '@/lib/day-agenda';

// Default visible window and grid density. The grid auto-expands beyond 8–18 to fit
// any item (or the current time) outside the window; see `timelineBounds`.
export const DEFAULT_START_HOUR = 8;
export const DEFAULT_END_HOUR = 18;
// 1.2px/min (owner-tuned, specs/0041 WS6 follow-up): enough grid room that a ~28-min
// meeting reaches MIN_ITEM_PX naturally — half-hour meetings render at true height and
// push-down only engages below that. Bubbles keep their full-content height; the grid,
// not the bubble chrome, absorbs short meetings.
export const PX_PER_HOUR = 72;
/** Floor height for a block so its title row + actions never clip. */
export const MIN_ITEM_PX = 34;

export interface TimelineBounds {
  /** First hour shown (integer, local). */
  startHour: number;
  /** One past the last hour shown (integer, local); `endHour - startHour` = grid hours. */
  endHour: number;
}

/** Fractional local hour of an ISO instant (e.g. 09:30 → 9.5); null when unparseable. */
function localHour(iso: string | null): number | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return d.getHours() + d.getMinutes() / 60;
}

/** Fractional local hour of a `Date`. */
function dateHour(d: Date): number {
  return d.getHours() + d.getMinutes() / 60;
}

/**
 * Grid bounds: the default 8–18 window, widened (integer hours) to include any
 * non-dismissed item's start/end and — when `includeNow` — the current time, clamped
 * to [0, 24]. A duration-less item is treated as ~30 min so its block doesn't push the
 * end hour. `includeNow` is true for the today view (keep the now-line visible) and
 * false when navigating to another day (specs/0038 WS4): there's no now-line on a
 * different day, so the grid should only fit that day's items, not stretch to the
 * current wall-clock hour.
 */
export function timelineBounds(
  items: DayAgendaItem[],
  now: Date = new Date(),
  includeNow = true,
): TimelineBounds {
  let startHour = DEFAULT_START_HOUR;
  let endHour = DEFAULT_END_HOUR;

  for (const it of items) {
    if (it.dismissed) continue;
    const s = localHour(it.startTime);
    if (s != null) startHour = Math.min(startHour, Math.floor(s));
    const e = it.endTime ? localHour(it.endTime) : s == null ? null : s + 0.5;
    if (e != null) endHour = Math.max(endHour, Math.ceil(e));
  }

  // Keep the "now" line inside the grid (today view only).
  if (includeNow) {
    const nh = dateHour(now);
    startHour = Math.min(startHour, Math.floor(nh));
    endHour = Math.max(endHour, Math.ceil(nh));
  }

  startHour = Math.max(0, startHour);
  endHour = Math.min(24, Math.max(endHour, startHour + 1));
  return { startHour, endHour };
}

/** Total grid height in px. */
export function timelineHeightPx(bounds: TimelineBounds, pxPerHour: number = PX_PER_HOUR): number {
  return (bounds.endHour - bounds.startHour) * pxPerHour;
}

/** Px offset from the top of the grid for a given fractional hour. */
export function hourOffsetPx(
  hour: number,
  bounds: TimelineBounds,
  pxPerHour: number = PX_PER_HOUR,
): number {
  return (hour - bounds.startHour) * pxPerHour;
}

/** Top px offset of an item's block (clamped to the grid top; 0 when start unparseable). */
export function itemOffsetPx(
  item: DayAgendaItem,
  bounds: TimelineBounds,
  pxPerHour: number = PX_PER_HOUR,
): number {
  const s = localHour(item.startTime);
  if (s == null) return 0;
  return Math.max(0, (s - bounds.startHour) * pxPerHour);
}

/** Block height in px from the item's duration, floored at `minPx`. */
export function itemHeightPx(
  item: DayAgendaItem,
  pxPerHour: number = PX_PER_HOUR,
  minPx: number = MIN_ITEM_PX,
): number {
  const s = localHour(item.startTime);
  const e = localHour(item.endTime);
  if (s == null || e == null || e <= s) return minPx;
  return Math.max(minPx, (e - s) * pxPerHour);
}

/** Px offset of the "now" indicator line within the grid. */
export function nowOffsetPx(
  now: Date,
  bounds: TimelineBounds,
  pxPerHour: number = PX_PER_HOUR,
): number {
  return (dateHour(now) - bounds.startHour) * pxPerHour;
}

// ---------------------------------------------------------------------------
// Phase + visual state
// ---------------------------------------------------------------------------

export type TimelinePhase = 'past' | 'now' | 'upcoming';

/**
 * Time phase of an item. A recording is always "past" (a completed artifact) unless it
 * is the live session (`recordingThisId`), which is "now" — mirroring `DayAgenda`'s
 * rule so a just-finished recording doesn't linger in the "now" grace window. Calendar
 * items defer to `agendaPhase`.
 */
export function itemPhase(
  item: DayAgendaItem,
  now: Date,
  recordingThisId: string | null,
): TimelinePhase {
  if (item.source === 'recording') {
    return recordingThisId && item.id === recordingThisId ? 'now' : 'past';
  }
  return agendaPhase(item, now);
}

export type TimelineVisualState =
  | 'recording' // the live session
  | 'now-joinable' // happening now, has a join link, can Join & Record
  | 'now' // happening now, not joinable
  | 'past-recorded' // ended, has a recording to open
  | 'past-unrecorded' // ended calendar event, never recorded
  | 'upcoming'; // not yet started

export interface TimelineContext {
  now: Date;
  isRecording: boolean;
  recordingThisId: string | null;
}

/** Visual bucket for an item's block styling. */
export function itemVisualState(item: DayAgendaItem, ctx: TimelineContext): TimelineVisualState {
  const { now, isRecording, recordingThisId } = ctx;
  if (recordingThisId && item.id === recordingThisId) return 'recording';
  const phase = itemPhase(item, now, recordingThisId);
  if (phase === 'now') {
    const canJoin = canJoinAgendaItem({
      hasZoomUrl: !!item.zoomUrl,
      recorded: item.status.recorded,
      isRecordingThis: false,
      anyRecordingInProgress: isRecording,
    });
    return canJoin ? 'now-joinable' : 'now';
  }
  if (phase === 'past') {
    return item.status.recorded || item.meetingId ? 'past-recorded' : 'past-unrecorded';
  }
  return 'upcoming';
}

// ---------------------------------------------------------------------------
// Click routing (pure decision; the component performs the invoke/navigation)
// ---------------------------------------------------------------------------

export type ItemRoute =
  | { kind: 'return' } // this is the live recording → return to /record
  | { kind: 'open'; meetingId: string; tab?: 'prep' } // recorded → /meeting-details (`tab`
  //   set only for an unrecorded manual entry, whose Prep tab opens directly)
  | {
      // an unrecorded calendar occurrence (upcoming, happening-now, or past) → ensure a
      // scheduled row, open its Prep tab. Join & Record is a separate explicit button, not
      // the body-click action, so clicking a live meeting lets you look before you join.
      kind: 'prep';
      calendarEventId: string;
      seriesKey: string | null;
      title: string;
      occurrenceStart: string;
    }
  | { kind: 'none' }; // nothing actionable

/**
 * URL for an `{ kind: 'open' }` route (specs/0069b followup). The one spot that turns
 * that route shape into an actual path, so a clicked timeline item and a freshly
 * created manual meeting land on the identical URL — `tab: 'prep'` for an unrecorded
 * manual entry, no `tab` otherwise — instead of two call sites hand-building the string
 * and drifting apart.
 */
export function openMeetingUrl(meetingId: string, tab?: 'prep'): string {
  return tab ? `/meeting-details?id=${meetingId}&tab=${tab}` : `/meeting-details?id=${meetingId}`;
}

function prepRoute(item: DayAgendaItem): ItemRoute {
  return {
    kind: 'prep',
    calendarEventId: item.id,
    seriesKey: item.seriesKey ?? null,
    title: item.title,
    occurrenceStart: item.startTime,
  };
}

/**
 * What a click on a timeline item's BODY should do (Join & Record is a separate
 * explicit button — see {@link canJoinItem} — so a body click never auto-joins):
 *  - the live recording → return to the recorder;
 *  - a recorded meeting → open its details;
 *  - any other calendar occurrence (upcoming, happening-now, or past-unrecorded) →
 *    ensure a `scheduled` meeting and open its Prep tab, so you can look before joining;
 *  - otherwise nothing.
 */
export function routeForItem(item: DayAgendaItem, ctx: TimelineContext): ItemRoute {
  const { recordingThisId } = ctx;
  if (recordingThisId && item.id === recordingThisId) return { kind: 'return' };

  // A manual entry's `meetingId` is set from the moment it's created — it IS a
  // meeting row, not something the click has to ensure into existence — so the
  // `item.meetingId` branch below would otherwise fire immediately and open its
  // (empty) details instead of letting you prep it. Route to Prep directly (no
  // `api_ensure_scheduled_meeting` round-trip) until it's actually been recorded, at
  // which point it falls through to the ordinary details route (specs/0069 W3).
  if (item.source === 'manual' && item.meetingId && !item.status.recorded) {
    return { kind: 'open', meetingId: item.meetingId, tab: 'prep' };
  }

  // Anything already recorded → its details.
  if (item.meetingId) return { kind: 'open', meetingId: item.meetingId };

  // Any unrecorded calendar occurrence → Prep (ensure is idempotent by
  // (calendar_event_id, occurrence day)). Includes happening-now meetings: you view
  // first, then use the explicit Join & Record button when ready.
  if (item.source === 'calendar') return prepRoute(item);

  return { kind: 'none' };
}

/**
 * Whether a timeline item should show an explicit "Join & Record" button — a
 * happening-now, joinable (has a Zoom link, not already recorded/recording) occurrence.
 * Mirrors the `now-joinable` visual state.
 */
export function canJoinItem(item: DayAgendaItem, ctx: TimelineContext): boolean {
  const { now, isRecording, recordingThisId } = ctx;
  if (itemPhase(item, now, recordingThisId) !== 'now') return false;
  return canJoinAgendaItem({
    hasZoomUrl: !!item.zoomUrl,
    recorded: item.status.recorded,
    isRecordingThis: false,
    anyRecordingInProgress: isRecording,
  });
}

/**
 * Whether a timeline item can be edited or deleted in place (specs/0069 W3): a
 * manually added entry (no calendar event, no recording behind it) that hasn't been
 * recorded yet. The backend refuses to edit/delete a manual row once it's been
 * recorded — "edit it from the meeting page instead" — so the menu shouldn't offer
 * an action it knows will be rejected.
 */
export function canEditManualItem(item: DayAgendaItem): boolean {
  return item.source === 'manual' && !item.status.recorded;
}

/**
 * Whether a timeline item should show an explicit "Record" button (specs/0069 W3): a
 * manual entry, not yet recorded, with no recording already in progress anywhere.
 *
 * No phase gate (fix round 2, specs/0069 followup a): this used to require the "now"
 * phase, which left an entry scheduled more than `PRE_START_GRACE_MS` out with NO way to
 * record it against its own row — the only visible affordance was the transport rail's
 * REC key, which creates an unrelated ad-hoc meeting and strands the entry's prep. The
 * gate existed because pressing Record threaded the entry's *scheduled* start through
 * `joinAndRecord` unconditionally, and the backend (`meetings/commands.rs`) calls
 * `redate_scheduled_meeting` to that occurrence before promoting the row — recording a
 * future entry would have re-dated it forward and filed the recording under a day that
 * hasn't happened yet. `handleRecordManual` (in `page.tsx`) now sends the actual `now`
 * as the start when recording early, and only the scheduled start once it has passed
 * (matching Join & Record's calendar behavior, specs/0015) — so recording early can no
 * longer misdate the row, and a manual entry can offer Record from the moment it exists.
 */
export function canRecordManualItem(item: DayAgendaItem, ctx: TimelineContext): boolean {
  return item.source === 'manual' && !item.status.recorded && !ctx.isRecording;
}

// ---------------------------------------------------------------------------
// Overlap lanes
// ---------------------------------------------------------------------------

export interface LaidOutItem {
  item: DayAgendaItem;
  /** 0-based column within this item's overlap cluster. */
  lane: number;
  /** Number of columns the cluster was split into (block width = 1 / laneCount). */
  laneCount: number;
}

/**
 * True time interval of an item in fractional local hours, for lane assignment.
 * End-exclusive by convention (an 11:00 end never collides with an 11:00 start).
 * **Duration-less convention:** an item with no parseable end (or `end <= start`,
 * i.e. truly zero-length) is treated as lasting its *rendered minimum*
 * (`minPx / pxPerHour` hours) — its true extent is unknown but it will occupy at
 * least that much screen, so anything shorter would let a genuinely-overlapping
 * neighbour skip the lane split. A real-but-short meeting (e.g. 5 min) keeps its
 * true interval; its visual crowding is resolved by `layoutTimeline`'s push-down,
 * not by lanes. Falls back to `{0, min}` when the start is unparseable (mirroring
 * `itemOffsetPx`'s 0 clamp).
 */
function trueIntervalHours(
  item: DayAgendaItem,
  pxPerHour: number,
  minPx: number,
): { start: number; end: number } {
  const minHours = minPx / pxPerHour;
  const s = localHour(item.startTime);
  if (s == null) return { start: 0, end: minHours };
  const e = localHour(item.endTime);
  if (e == null || e <= s) return { start: s, end: s + minHours };
  return { start: s, end: e };
}

/**
 * Pack items into side-by-side lanes so *truly* overlapping blocks don't paint on top
 * of each other. Overlap is measured on the TRUE time interval `[start, end)` —
 * end-exclusive, so back-to-back meetings (11:00 end / 11:00 start) never lane-split
 * (specs/0041 WS6). The min-height render floor plays no part here; the visual
 * crowding it causes is resolved by `layoutTimeline`'s push-down instead of lanes.
 * Duration-less items follow `trueIntervalHours`'s rendered-minimum convention.
 * Non-overlapping items each get `lane 0 / laneCount 1` (full width). Returned
 * sorted by start time.
 */
export function assignLanes(
  items: DayAgendaItem[],
  pxPerHour: number = PX_PER_HOUR,
  minPx: number = MIN_ITEM_PX,
): LaidOutItem[] {
  const ranged = items
    .map((item) => ({ item, ...trueIntervalHours(item, pxPerHour, minPx) }))
    .sort((a, b) => a.start - b.start || a.end - b.end);

  const out: LaidOutItem[] = [];
  let cluster: { entry: (typeof ranged)[number]; lane: number }[] = [];
  let clusterEnd = -Infinity;
  let laneEnds: number[] = []; // last `end` (hours) per lane in the current cluster

  const flush = () => {
    const laneCount = Math.max(1, laneEnds.length);
    for (const c of cluster) out.push({ item: c.entry.item, lane: c.lane, laneCount });
    cluster = [];
    laneEnds = [];
    clusterEnd = -Infinity;
  };

  for (const entry of ranged) {
    if (entry.start >= clusterEnd && cluster.length > 0) flush();
    // First lane whose previous interval has ended (end-exclusive); else a new lane.
    let lane = laneEnds.findIndex((end) => end <= entry.start);
    if (lane === -1) {
      lane = laneEnds.length;
      laneEnds.push(entry.end);
    } else {
      laneEnds[lane] = entry.end;
    }
    cluster.push({ entry, lane });
    clusterEnd = Math.max(clusterEnd, entry.end);
  }
  if (cluster.length > 0) flush();

  return out;
}

export interface PositionedTimelineItem extends LaidOutItem {
  /** Rendered top px — the true start offset, possibly pushed down (see `layoutTimeline`). */
  top: number;
  /** Rendered height px — floored at `minPx`, otherwise ending at the true end time. */
  height: number;
}

/**
 * Full render geometry for the day timeline: lanes from TRUE times (`assignLanes`),
 * then per-item top/height with the "stack, don't stagger" rule (specs/0041 WS6).
 *
 * Each block's natural geometry is its true start offset and true duration floored at
 * `minPx`. When a floored block would paint over the next (truly non-overlapping)
 * block, the follower's TOP is pushed down to the floored bottom instead of splitting
 * lanes — so back-to-back short meetings stack contiguously. A pushed block keeps its
 * true rendered *bottom* (its height shrinks by the push, never below `minPx`), so the
 * rendered bottom never implies more duration than the floor requires. Pushes cascade
 * (three back-to-back 5-min meetings stack as three `minPx` blocks) but each pairwise
 * push is bounded by the predecessor's floor delta, since only floor inflation can
 * make a truly-earlier block's rendered bottom cross a follower's start. Push-down
 * only considers blocks whose lane columns horizontally intersect, so side-by-side
 * true overlaps are unaffected.
 */
export function layoutTimeline(
  items: DayAgendaItem[],
  bounds: TimelineBounds,
  pxPerHour: number = PX_PER_HOUR,
  minPx: number = MIN_ITEM_PX,
): PositionedTimelineItem[] {
  const laned = assignLanes(items, pxPerHour, minPx); // sorted by true start
  const placed: { x0: number; x1: number; bottom: number }[] = [];
  const out: PositionedTimelineItem[] = [];

  for (const l of laned) {
    const s = localHour(l.item.startTime);
    const e = localHour(l.item.endTime);
    // Same as itemOffsetPx, from the already-parsed start (no re-parse).
    const naturalTop = s == null ? 0 : Math.max(0, (s - bounds.startHour) * pxPerHour);
    const truePx = s != null && e != null && e > s ? (e - s) * pxPerHour : 0;
    // Horizontal span of this block's lane column, as a fraction of the track width.
    const x0 = l.lane / l.laneCount;
    const x1 = (l.lane + 1) / l.laneCount;

    let top = naturalTop;
    for (const p of placed) {
      if (p.x0 < x1 && x0 < p.x1) top = Math.max(top, p.bottom);
    }
    const height = Math.max(minPx, naturalTop + truePx - top);
    placed.push({ x0, x1, bottom: top + height });
    out.push({ ...l, top, height });
  }

  return out;
}

/** "8 AM" / "12 PM" hour-gutter label for a 24h hour value. */
export function hourLabel(hour: number): string {
  const h = ((hour % 24) + 24) % 24;
  const period = h < 12 ? 'AM' : 'PM';
  const display = h % 12 === 0 ? 12 : h % 12;
  return `${display} ${period}`;
}

// ---------------------------------------------------------------------------
// Local-day navigation (specs/0038 WS4)
//
// The agenda's day boundaries are LOCAL, so every key is built from local date
// parts — never `toISOString()`, which is UTC and can land on the wrong calendar
// day near midnight. A `dateKey` is a `YYYY-MM-DD` string in the machine's local
// timezone, exactly what `api_get_day_agenda(date)` expects.
// ---------------------------------------------------------------------------

/** Local `YYYY-MM-DD` for a `Date` (from LOCAL parts, never UTC `toISOString`). */
export function localDateKey(d: Date = new Date()): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

/** Parse a `YYYY-MM-DD` key to a local `Date` at local midnight. */
export function parseLocalDateKey(key: string): Date {
  const [y, m, d] = key.split('-').map(Number);
  return new Date(y, (m || 1) - 1, d || 1);
}

/** A `YYYY-MM-DD` key shifted by `deltaDays` local calendar days (DST-safe). */
export function shiftDateKey(key: string, deltaDays: number): string {
  const d = parseLocalDateKey(key);
  d.setDate(d.getDate() + deltaDays);
  return localDateKey(d);
}

/** Whether a `YYYY-MM-DD` key is today's local date. */
export function isTodayKey(key: string, now: Date = new Date()): boolean {
  return key === localDateKey(now);
}

/**
 * Human-friendly day label for the header: "Today" / "Tomorrow" / "Yesterday" for the
 * three days around now, else a formatted date like "Mon, Jul 7".
 */
export function dayLabel(key: string, now: Date = new Date()): string {
  const todayKey = localDateKey(now);
  if (key === todayKey) return 'Today';
  if (key === shiftDateKey(todayKey, 1)) return 'Tomorrow';
  if (key === shiftDateKey(todayKey, -1)) return 'Yesterday';
  return parseLocalDateKey(key).toLocaleDateString([], {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
  });
}

/**
 * The seven `YYYY-MM-DD` keys of the Monday-started week that contains `key`
 * (specs/0038 WS4 week view). Monday-first matches how work weeks read.
 */
export function weekDaysFor(key: string): string[] {
  const d = parseLocalDateKey(key);
  const mondayOffset = (d.getDay() + 6) % 7; // Sun=0 → 6, Mon=1 → 0, …
  const monday = shiftDateKey(key, -mondayOffset);
  return Array.from({ length: 7 }, (_, i) => shiftDateKey(monday, i));
}
