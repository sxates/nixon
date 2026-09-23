import { describe, it, expect } from 'vitest';
import {
  timelineBounds,
  timelineHeightPx,
  itemOffsetPx,
  itemHeightPx,
  nowOffsetPx,
  hourOffsetPx,
  itemPhase,
  itemVisualState,
  routeForItem,
  canJoinItem,
  canEditManualItem,
  canDeleteRecordedItem,
  canRecordManualItem,
  findRecordingRowId,
  assignLanes,
  layoutTimeline,
  hourLabel,
  localDateKey,
  parseLocalDateKey,
  shiftDateKey,
  isTodayKey,
  dayLabel,
  weekDaysFor,
  DEFAULT_START_HOUR,
  DEFAULT_END_HOUR,
  PX_PER_HOUR,
  MIN_ITEM_PX,
  type TimelineContext,
} from '@/lib/today-timeline';
import type { DayAgendaItem } from '@/lib/day-agenda';

// specs/0036 WS7 — pure layout math + click-routing for the Today timeline. Times are
// built from LOCAL-time components then serialized to ISO, so `localHour`/`getHours()`
// round-trip to the intended clock hour regardless of the machine timezone.
function iso(hour: number, minute = 0): string {
  return new Date(2026, 6, 4, hour, minute, 0, 0).toISOString();
}

function item(partial: Partial<DayAgendaItem>): DayAgendaItem {
  return {
    id: 'evt-1',
    title: 'Weekly Design Review',
    startTime: iso(10),
    endTime: iso(11),
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: null,
    status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    seriesKey: null,
    ...partial,
  };
}

const ctx = (over: Partial<TimelineContext> = {}): TimelineContext => ({
  now: new Date(2026, 6, 4, 12, 0),
  isRecording: false,
  recordingThisId: null,
  ...over,
});

/** specs/0069 W3 — a manually added timeline entry (no calendar event behind it). */
function manualItem(
  partial: Partial<DayAgendaItem> & { id?: string; recorded?: boolean } = {},
): DayAgendaItem {
  const { recorded, id = 'meeting-1', ...rest } = partial;
  return item({
    id,
    source: 'manual',
    meetingId: id,
    calendarEventId: `nixon-manual:${id}`,
    status: {
      recorded: !!recorded,
      transcribed: false,
      summarized: false,
      speakersIdentified: false,
    },
    ...rest,
  });
}

describe('timelineBounds', () => {
  it('defaults to the 8–18 window when everything fits (and now is inside)', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const bounds = timelineBounds([item({ startTime: iso(10), endTime: iso(11) })], now);
    expect(bounds).toEqual({ startHour: DEFAULT_START_HOUR, endHour: DEFAULT_END_HOUR });
  });

  it('expands the start hour to fit an item before 8am', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const bounds = timelineBounds([item({ startTime: iso(6, 30), endTime: iso(7, 15) })], now);
    expect(bounds.startHour).toBe(6);
    expect(bounds.endHour).toBe(DEFAULT_END_HOUR);
  });

  it('expands the end hour to fit an item running past 6pm', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const bounds = timelineBounds([item({ startTime: iso(17, 30), endTime: iso(19, 15) })], now);
    expect(bounds.startHour).toBe(DEFAULT_START_HOUR);
    expect(bounds.endHour).toBe(20); // ceil(19.25)
  });

  it('expands to keep the now-line in view when now is outside 8–18', () => {
    const now = new Date(2026, 6, 4, 20, 30);
    const bounds = timelineBounds([item({ startTime: iso(9), endTime: iso(10) })], now);
    expect(bounds.startHour).toBe(DEFAULT_START_HOUR);
    expect(bounds.endHour).toBe(21); // ceil(20.5)
  });

  it('ignores dismissed items when computing bounds', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const bounds = timelineBounds(
      [item({ startTime: iso(6), endTime: iso(7), dismissed: true })],
      now,
    );
    expect(bounds.startHour).toBe(DEFAULT_START_HOUR);
  });

  it('does NOT stretch to the current hour when includeNow is false (specs/0038 WS4)', () => {
    // Viewing another day: now is at 20:30 but the grid should fit only the item.
    const now = new Date(2026, 6, 4, 20, 30);
    const bounds = timelineBounds([item({ startTime: iso(9), endTime: iso(10) })], now, false);
    expect(bounds).toEqual({ startHour: DEFAULT_START_HOUR, endHour: DEFAULT_END_HOUR });
  });
});

// specs/0038 WS4 — local-day navigation helpers (timezone-correct, built from local
// date parts, never UTC toISOString).
describe('date navigation helpers', () => {
  it('builds a YYYY-MM-DD key from local parts', () => {
    expect(localDateKey(new Date(2026, 6, 7, 23, 30))).toBe('2026-07-07');
    expect(localDateKey(new Date(2026, 0, 1, 0, 0))).toBe('2026-01-01');
  });

  it('round-trips a key through parse → localDateKey', () => {
    expect(localDateKey(parseLocalDateKey('2026-07-07'))).toBe('2026-07-07');
  });

  it('shifts across month and year boundaries', () => {
    expect(shiftDateKey('2026-07-07', 1)).toBe('2026-07-08');
    expect(shiftDateKey('2026-07-07', -1)).toBe('2026-07-06');
    expect(shiftDateKey('2026-07-31', 1)).toBe('2026-08-01');
    expect(shiftDateKey('2026-12-31', 1)).toBe('2027-01-01');
  });

  it('detects today from a reference now', () => {
    const now = new Date(2026, 6, 7, 9, 0);
    expect(isTodayKey('2026-07-07', now)).toBe(true);
    expect(isTodayKey('2026-07-08', now)).toBe(false);
  });

  it('labels today / tomorrow / yesterday, else a formatted date', () => {
    const now = new Date(2026, 6, 7, 9, 0);
    expect(dayLabel('2026-07-07', now)).toBe('Today');
    expect(dayLabel('2026-07-08', now)).toBe('Tomorrow');
    expect(dayLabel('2026-07-06', now)).toBe('Yesterday');
    expect(dayLabel('2026-07-13', now)).toBe(
      parseLocalDateKey('2026-07-13').toLocaleDateString([], {
        weekday: 'short',
        month: 'short',
        day: 'numeric',
      }),
    );
  });

  it('returns the seven Monday-started days of the containing week', () => {
    // 2026-07-07 is a Tuesday → week runs Mon 6th … Sun 12th.
    expect(weekDaysFor('2026-07-07')).toEqual([
      '2026-07-06',
      '2026-07-07',
      '2026-07-08',
      '2026-07-09',
      '2026-07-10',
      '2026-07-11',
      '2026-07-12',
    ]);
    // A Sunday still maps back to the Monday that starts its week.
    expect(weekDaysFor('2026-07-12')[0]).toBe('2026-07-06');
  });
});

describe('offset + height math', () => {
  const bounds = { startHour: 8, endHour: 18 };

  it('positions an item block by its start relative to the grid top', () => {
    // 10:30 local → (10.5 - 8) * scale
    expect(itemOffsetPx(item({ startTime: iso(10, 30) }), bounds)).toBe(2.5 * PX_PER_HOUR);
  });

  it('clamps an item starting before the grid to offset 0', () => {
    expect(itemOffsetPx(item({ startTime: iso(7) }), bounds)).toBe(0);
  });

  it('derives height from duration', () => {
    // 10:00 → 11:00 = 1h = 60px
    expect(itemHeightPx(item({ startTime: iso(10), endTime: iso(11) }))).toBe(PX_PER_HOUR);
  });

  it('floors height at MIN_ITEM_PX for a short or duration-less item', () => {
    expect(itemHeightPx(item({ startTime: iso(10), endTime: iso(10, 5) }))).toBe(MIN_ITEM_PX);
    expect(itemHeightPx(item({ startTime: iso(10), endTime: null }))).toBe(MIN_ITEM_PX);
  });

  it('computes the now-line offset and total grid height', () => {
    const now = new Date(2026, 6, 4, 9, 0);
    expect(nowOffsetPx(now, bounds)).toBe(PX_PER_HOUR); // (9 - 8) * scale
    expect(timelineHeightPx(bounds)).toBe(10 * PX_PER_HOUR); // 10h window
    expect(hourOffsetPx(12, bounds)).toBe(4 * PX_PER_HOUR);
  });
});

describe('itemPhase', () => {
  it('treats a non-live recording as past even within the grace window', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const rec = item({ source: 'recording', startTime: iso(11, 30), endTime: null, meetingId: 'm1' });
    expect(itemPhase(rec, now, null)).toBe('past');
  });

  it('treats the live recording as now', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const rec = item({ id: 'live', source: 'recording', startTime: iso(11, 55), meetingId: 'm1' });
    expect(itemPhase(rec, now, 'live')).toBe('now');
  });
});

describe('itemVisualState', () => {
  it('flags a happening-now joinable event', () => {
    const now = new Date(2026, 6, 4, 10, 15);
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(itemVisualState(it, ctx({ now }))).toBe('now-joinable');
  });

  it('distinguishes past-recorded from past-unrecorded', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const recorded = item({
      startTime: iso(9),
      endTime: iso(10),
      meetingId: 'm1',
      status: { recorded: true, transcribed: true, summarized: false, speakersIdentified: false },
    });
    const missed = item({ startTime: iso(9), endTime: iso(10) });
    expect(itemVisualState(recorded, ctx({ now }))).toBe('past-recorded');
    expect(itemVisualState(missed, ctx({ now }))).toBe('past-unrecorded');
  });

  // Owner report 2026-09-23: three meetings added in Nixon and never recorded read
  // "RECORDED" on Today once their time passed. A manual entry always carries its own
  // placeholder `meetingId`, which the `|| item.meetingId` fallback took as a recording.
  it('a past meeting added in Nixon and never recorded is not "recorded"', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const it = manualItem({ startTime: iso(9), endTime: iso(10) });
    expect(itemVisualState(it, ctx({ now }))).toBe('past-unrecorded');
  });

  it('a recorded meeting added in Nixon still reads recorded', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const it = manualItem({ startTime: iso(9), endTime: iso(10), recorded: true });
    expect(itemVisualState(it, ctx({ now }))).toBe('past-recorded');
  });

  it('flags the live recording', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const live = item({ id: 'live', source: 'recording', startTime: iso(11, 55), meetingId: 'm1' });
    expect(itemVisualState(live, ctx({ now, isRecording: true, recordingThisId: 'live' }))).toBe(
      'recording',
    );
  });
});

describe('routeForItem', () => {
  it('routes an upcoming calendar occurrence to ensure-scheduled + Prep', () => {
    const now = new Date(2026, 6, 4, 9, 0);
    const it = item({
      id: 'evt-abc',
      startTime: iso(14),
      endTime: iso(15),
      seriesKey: 'ical-series-9',
      title: 'Leadership Sync',
    });
    expect(routeForItem(it, ctx({ now }))).toEqual({
      kind: 'prep',
      calendarEventId: 'evt-abc',
      seriesKey: 'ical-series-9',
      title: 'Leadership Sync',
      occurrenceStart: iso(14),
    });
  });

  it('routes a recorded meeting to its details', () => {
    const now = new Date(2026, 6, 4, 18, 0);
    const it = item({
      startTime: iso(9),
      endTime: iso(10),
      meetingId: 'm-42',
      status: { recorded: true, transcribed: true, summarized: true, speakersIdentified: true },
    });
    expect(routeForItem(it, ctx({ now }))).toEqual({ kind: 'open', meetingId: 'm-42' });
  });

  it('routes a happening-now joinable event to Prep (view), NOT auto-join', () => {
    // The body click views the meeting; Join & Record is a separate explicit button.
    const now = new Date(2026, 6, 4, 10, 10);
    const it = item({ id: 'evt-now', startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(routeForItem(it, ctx({ now }))).toMatchObject({ kind: 'prep', calendarEventId: 'evt-now' });
  });

  it('routes a happening-now non-joinable event to Prep too', () => {
    const now = new Date(2026, 6, 4, 10, 10);
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(routeForItem(it, ctx({ now, isRecording: true }))).toMatchObject({ kind: 'prep' });
  });

  it('returns to the recorder for the live session', () => {
    const now = new Date(2026, 6, 4, 12, 0);
    const live = item({ id: 'live', source: 'recording', startTime: iso(11, 55), meetingId: 'm1' });
    expect(routeForItem(live, ctx({ now, isRecording: true, recordingThisId: 'live' }))).toEqual({
      kind: 'return',
    });
  });

  it('routes a past unrecorded calendar occurrence to Prep to fill in', () => {
    const now = new Date(2026, 6, 4, 16, 0);
    const it = item({ id: 'evt-past', startTime: iso(9), endTime: iso(10) });
    expect(routeForItem(it, ctx({ now }))).toMatchObject({ kind: 'prep', calendarEventId: 'evt-past' });
  });

  it('opens a manual entry on its Prep tab, not its details (specs/0069 W3)', () => {
    const now = new Date(2026, 6, 4, 9, 0);
    const it = manualItem({ id: 'meeting-1', startTime: iso(14) });
    expect(routeForItem(it, ctx({ now }))).toEqual({
      kind: 'open',
      meetingId: 'meeting-1',
      tab: 'prep',
    });
  });

  it('opens a manual entry that has been recorded on its details (specs/0069 W3)', () => {
    const now = new Date(2026, 6, 4, 9, 0);
    const it = manualItem({ id: 'meeting-1', recorded: true });
    expect(routeForItem(it, ctx({ now }))).toEqual({ kind: 'open', meetingId: 'meeting-1' });
  });
});

describe('canEditManualItem (specs/0069 W3)', () => {
  it('is true for an unrecorded manual entry', () => {
    expect(canEditManualItem(manualItem())).toBe(true);
  });

  it('is false once the entry has been recorded — the backend refuses the edit', () => {
    expect(canEditManualItem(manualItem({ recorded: true }))).toBe(false);
  });

  it('is false for a calendar event (nothing to edit — it belongs to the calendar)', () => {
    expect(canEditManualItem(item({ source: 'calendar' }))).toBe(false);
  });
});

describe('canRecordManualItem (specs/0069 W3, fix round 1 Finding 1; gate dropped in followup a)', () => {
  // The default manualItem() runs 10:00–11:00 (from item()'s defaults).
  const nowDuring = new Date(2026, 6, 4, 10, 30); // inside the window
  const nowBefore = new Date(2026, 6, 4, 9, 0); // well before start

  // Owner feedback 2026-09-23: dropping the phase gate (to allow recording EARLY) also left
  // Record on every added meeting after it had ended — never asked for.
  it('is false once the entry has ended', () => {
    const nowAfter = new Date(2026, 6, 4, 12, 0);
    expect(canRecordManualItem(manualItem(), ctx({ now: nowAfter }))).toBe(false);
  });

  it('is true for an unrecorded manual entry in its "now" window when nothing else is recording', () => {
    expect(canRecordManualItem(manualItem(), ctx({ now: nowDuring }))).toBe(true);
  });

  it('is true for a manual entry that has not started yet (fix round 2, followup a)', () => {
    // No phase gate any more: `handleRecordManual` now sends `now` (not the future
    // scheduled start) as `startsAt` when recording early, so there's no more
    // `redate_scheduled_meeting`-into-the-future risk this used to guard against. The
    // only affordance without this button was the transport rail's REC key, which
    // creates an unrelated ad-hoc meeting and strands the entry's prep.
    expect(canRecordManualItem(manualItem(), ctx({ now: nowBefore }))).toBe(true);
  });

  it('is true well outside the old pre-start grace window too', () => {
    // Starts at 14:00; 13:00 is an hour early — well past the old 5-minute grace.
    const it2 = manualItem({ startTime: iso(14), endTime: iso(15) });
    const now = new Date(2026, 6, 4, 13, 0);
    expect(canRecordManualItem(it2, ctx({ now }))).toBe(true);
  });

  it('is false once the entry has been recorded', () => {
    expect(canRecordManualItem(manualItem({ recorded: true }), ctx({ now: nowDuring }))).toBe(false);
  });

  it('is false while another recording is in progress', () => {
    expect(canRecordManualItem(manualItem(), ctx({ now: nowDuring, isRecording: true }))).toBe(
      false,
    );
  });

  it('is false for a calendar event even if unrecorded and in its "now" window', () => {
    expect(canRecordManualItem(item({ source: 'calendar' }), ctx({ now: nowDuring }))).toBe(false);
  });
});

describe('canJoinItem', () => {
  it('is true for a happening-now joinable event (drives the Join & Record button)', () => {
    const now = new Date(2026, 6, 4, 10, 10);
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(canJoinItem(it, ctx({ now }))).toBe(true);
  });

  it('is true 5 min before the official start (pre-start grace)', () => {
    const now = new Date(2026, 6, 4, 9, 56); // 4 min before a 10:00 start (same local basis as iso())
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(canJoinItem(it, ctx({ now }))).toBe(true);
  });

  it('is false for an upcoming event outside the grace window', () => {
    const now = new Date(2026, 6, 4, 9, 0);
    const it = item({ startTime: iso(14), endTime: iso(15), zoomUrl: 'https://zoom.us/j/1' });
    expect(canJoinItem(it, ctx({ now }))).toBe(false);
  });

  it('is false without a Zoom link', () => {
    const now = new Date(2026, 6, 4, 10, 10);
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: null });
    expect(canJoinItem(it, ctx({ now }))).toBe(false);
  });

  it('is false while another recording is in progress', () => {
    const now = new Date(2026, 6, 4, 10, 10);
    const it = item({ startTime: iso(10), endTime: iso(11), zoomUrl: 'https://zoom.us/j/1' });
    expect(canJoinItem(it, ctx({ now, isRecording: true }))).toBe(false);
  });
});

// specs/0041 WS6 — lanes come from TRUE time intervals ([start, end), end-exclusive),
// never from the min-height-inflated rendered pixels.
describe('assignLanes', () => {
  it('gives non-overlapping items full width (lane 0 / 1)', () => {
    const a = item({ id: 'a', startTime: iso(9), endTime: iso(10) });
    const b = item({ id: 'b', startTime: iso(11), endTime: iso(12) });
    const laid = assignLanes([a, b]);
    expect(laid.every((l) => l.lane === 0 && l.laneCount === 1)).toBe(true);
  });

  it('splits two overlapping items into two lanes', () => {
    const a = item({ id: 'a', startTime: iso(9), endTime: iso(10, 30) });
    const b = item({ id: 'b', startTime: iso(10), endTime: iso(11) });
    const laid = assignLanes([a, b]);
    expect(laid.map((l) => l.laneCount)).toEqual([2, 2]);
    expect(new Set(laid.map((l) => l.lane))).toEqual(new Set([0, 1]));
  });

  it('keeps back-to-back meetings (11:00 end / 11:00 start) full width — end-exclusive', () => {
    const a = item({ id: 'a', startTime: iso(10), endTime: iso(11) });
    const b = item({ id: 'b', startTime: iso(11), endTime: iso(12) });
    const laid = assignLanes([a, b]);
    expect(laid.every((l) => l.lane === 0 && l.laneCount === 1)).toBe(true);
  });

  it('does NOT lane a floored short meeting against its truly-later neighbour', () => {
    // 15 min renders at MIN_ITEM_PX (past its true end) but lanes use true times.
    const a = item({ id: 'a', startTime: iso(10), endTime: iso(10, 15) });
    const b = item({ id: 'b', startTime: iso(10, 15), endTime: iso(11, 15) });
    const laid = assignLanes([a, b]);
    expect(laid.every((l) => l.lane === 0 && l.laneCount === 1)).toBe(true);
  });

  it('treats a duration-less item as its rendered minimum for lane purposes', () => {
    // No end → synthetic MIN_ITEM_PX/PX_PER_HOUR hours (28 min): overlaps a 10:15 start…
    const a = item({ id: 'a', startTime: iso(10), endTime: null });
    const b = item({ id: 'b', startTime: iso(10, 15), endTime: iso(11) });
    expect(assignLanes([a, b]).map((l) => l.laneCount)).toEqual([2, 2]);
    // …but not a 10:30 start (synthetic end 10:28 < 10:30).
    const c = item({ id: 'c', startTime: iso(10, 30), endTime: iso(11) });
    expect(assignLanes([a, c]).every((l) => l.laneCount === 1)).toBe(true);
  });

  it('sorts output by start time', () => {
    const a = item({ id: 'a', startTime: iso(9) });
    const b = item({ id: 'b', startTime: iso(11) });
    const laid = assignLanes([b, a]);
    expect(laid.map((l) => l.item.id)).toEqual(['a', 'b']);
  });
});

// specs/0041 WS6 — full render geometry: floored blocks push the next block DOWN
// (stack, don't stagger) instead of lane-splitting; rendered bottoms never imply more
// duration than the MIN_ITEM_PX floor requires.
describe('layoutTimeline', () => {
  const bounds = { startHour: 8, endHour: 18 };
  // Scale-relative shorthand so these stay pinned to geometry, not to a px/hour value.
  const H = PX_PER_HOUR;

  it('renders untouched geometry for comfortably-spaced meetings', () => {
    const a = item({ id: 'a', startTime: iso(9), endTime: iso(10) });
    const b = item({ id: 'b', startTime: iso(11), endTime: iso(12) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect(la).toMatchObject({ lane: 0, laneCount: 1, top: H, height: H });
    expect(lb).toMatchObject({ lane: 0, laneCount: 1, top: 3 * H, height: H });
  });

  it('stacks back-to-back meetings full width with no push when nothing is floored', () => {
    const a = item({ id: 'a', startTime: iso(10), endTime: iso(11) });
    const b = item({ id: 'b', startTime: iso(11), endTime: iso(12) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect(la).toMatchObject({ laneCount: 1, top: 2 * H, height: H });
    expect(lb).toMatchObject({ laneCount: 1, top: 3 * H, height: H });
  });

  it('pushes the follower below a floored 11:00-adjacent bubble instead of laning', () => {
    // 10:45–11:00 is shorter than the floor; the 11:00 meeting slides down below the
    // floored bubble and keeps its TRUE bottom by shrinking, staying full width.
    const a = item({ id: 'a', startTime: iso(10, 45), endTime: iso(11) });
    const b = item({ id: 'b', startTime: iso(11), endTime: iso(12) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect(la).toMatchObject({ laneCount: 1, top: 2.75 * H, height: MIN_ITEM_PX });
    expect(lb).toMatchObject({ laneCount: 1, top: 2.75 * H + MIN_ITEM_PX });
    expect(lb.top + lb.height).toBe(4 * H); // true 12:00 end preserved
  });

  it('floors a sub-floor meeting and pushes (not lanes) the next block', () => {
    // 10 min at the current scale is below MIN_ITEM_PX.
    const a = item({ id: 'a', startTime: iso(10), endTime: iso(10, 10) });
    const b = item({ id: 'b', startTime: iso(10, 10), endTime: iso(11, 10) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect(la).toMatchObject({ laneCount: 1, top: 2 * H, height: MIN_ITEM_PX });
    expect(lb).toMatchObject({ laneCount: 1, top: 2 * H + MIN_ITEM_PX });
    expect(lb.top + lb.height).toBeCloseTo((2 + 7 / 6) * H, 6); // true 11:10 end preserved
    expect(lb.height).toBeGreaterThanOrEqual(MIN_ITEM_PX);
  });

  it('bounds a push-down cascade: three 5-min back-to-back meetings stack contiguously', () => {
    const a = item({ id: 'a', startTime: iso(9), endTime: iso(9, 5) });
    const b = item({ id: 'b', startTime: iso(9, 5), endTime: iso(9, 10) });
    const c = item({ id: 'c', startTime: iso(9, 10), endTime: iso(9, 15) });
    const laid = layoutTimeline([a, b, c], bounds);
    expect(laid.every((l) => l.lane === 0 && l.laneCount === 1)).toBe(true);
    expect(laid.map((l) => l.top)).toEqual([H, H + MIN_ITEM_PX, H + 2 * MIN_ITEM_PX]);
    expect(laid.every((l) => l.height === MIN_ITEM_PX)).toBe(true);
  });

  it('a typical short meeting no longer floors at the denser scale', () => {
    // 30 min = H/2 px ≥ MIN_ITEM_PX, so back-to-back half-hour meetings need no push.
    const a = item({ id: 'a', startTime: iso(10), endTime: iso(10, 30) });
    const b = item({ id: 'b', startTime: iso(10, 30), endTime: iso(11) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect(la).toMatchObject({ laneCount: 1, top: 2 * H, height: H / 2 });
    expect(lb).toMatchObject({ laneCount: 1, top: 2.5 * H, height: H / 2 });
  });

  it('still lanes TRUE overlaps side by side, with no cross-lane push', () => {
    const a = item({ id: 'a', startTime: iso(9), endTime: iso(10, 30) });
    const b = item({ id: 'b', startTime: iso(10), endTime: iso(11) });
    const [la, lb] = layoutTimeline([a, b], bounds);
    expect([la.laneCount, lb.laneCount]).toEqual([2, 2]);
    expect(la).toMatchObject({ top: H, height: 1.5 * H });
    expect(lb).toMatchObject({ top: 2 * H, height: H }); // unpushed: different lane
  });
});

describe('hourLabel', () => {
  it('formats 12-hour clock labels', () => {
    expect(hourLabel(8)).toBe('8 AM');
    expect(hourLabel(12)).toBe('12 PM');
    expect(hourLabel(13)).toBe('1 PM');
    expect(hourLabel(0)).toBe('12 AM');
  });
});

describe('findRecordingRowId', () => {
  const live = item({ id: 'row-live', source: 'recording', meetingId: 'm-live', title: 'Standup' });
  const other = item({ id: 'row-other', meetingId: 'm-other', title: 'Other' });

  it('is null when nothing is recording', () => {
    expect(findRecordingRowId([[live]], { isRecording: false, liveIds: ['m-live'], liveTitle: null })).toBeNull();
  });

  // Owner report 2026-09-23: no animated reels in the Week view while recording. The lookup
  // only searched the single day's `items`; Week draws from `weekItems`.
  it('finds the live meeting in any of the lists it is given', () => {
    const found = findRecordingRowId([[], [other], [live]], {
      isRecording: true,
      liveIds: ['m-live'],
      liveTitle: null,
    });
    expect(found).toBe('row-live');
  });

  it('prefers an id match, then falls back to the live title', () => {
    expect(
      findRecordingRowId([[other, live]], { isRecording: true, liveIds: [null, 'm-other'], liveTitle: 'Standup' }),
    ).toBe('row-other');
    expect(
      findRecordingRowId([[other, live]], { isRecording: true, liveIds: [null], liveTitle: ' standup ' }),
    ).toBe('row-live');
  });
});

// Owner feedback 2026-09-23: on List and Week only unrecorded meetings had a `…` menu. A
// recorded meeting now offers what All Meetings offers for it — Delete meeting.
describe('canDeleteRecordedItem', () => {
  const recorded = item({
    id: 'm1',
    source: 'recording',
    meetingId: 'm1',
    status: { recorded: true, transcribed: true, summarized: true, speakersIdentified: true },
  });

  it('allows a recorded meeting', () => {
    expect(canDeleteRecordedItem(recorded, ctx())).toBe(true);
  });

  it('never the meeting being recorded right now', () => {
    expect(canDeleteRecordedItem(recorded, ctx({ isRecording: true, recordingThisId: 'm1' }))).toBe(false);
  });

  it('not an unrecorded calendar event or an unrecorded manual entry — those have Hide / Edit', () => {
    expect(canDeleteRecordedItem(item({}), ctx())).toBe(false);
    expect(canDeleteRecordedItem(manualItem(), ctx())).toBe(false);
  });

  it('a manual entry that was recorded is a recorded meeting', () => {
    expect(canDeleteRecordedItem(manualItem({ recorded: true }), ctx())).toBe(true);
  });
});
