import { describe, it, expect, beforeEach } from 'vitest';
import {
  consumePendingJoinMeeting,
  findAgendaItemForOccurrence,
  peekPendingJoinMeeting,
} from '@/lib/calendar';
import type { DayAgendaItem } from '@/lib/day-agenda';

// Must match the private key in lib/calendar.ts (stashPendingJoinMeeting is not
// exported, so the test writes the sessionStorage entry the recorder consumes).
const PENDING_JOIN_MEETING_KEY = 'nixon-join-and-record-meeting';

// specs/0023 L3 — sessionStorage invariant tied to specs/0019 WS6.7. A Join & Record
// pre-creates a meeting and stashes it under this key; the recorder's start path
// consumes it. If consume did NOT clear the key, a stale pending meeting would leak
// into the *next* (unrelated) recording and attach it to the wrong calendar row.
describe('consumePendingJoinMeeting', () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it('returns the pending meeting and clears the key (no leak to the next session)', () => {
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: 'meeting-123', title: 'Weekly Sync' }),
    );

    // specs/0019 WS6.3 — the calendar link fields default to null when absent.
    expect(consumePendingJoinMeeting()).toEqual({
      id: 'meeting-123',
      title: 'Weekly Sync',
      calendarEventId: null,
      startsAt: null,
      // specs/0036: series key defaults to null when the stash predates / lacks it.
      calendarSeriesKey: null,
    });
    // The key is gone, so a subsequent recording sees nothing pending.
    expect(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY)).toBeNull();
    expect(consumePendingJoinMeeting()).toBeNull();
  });

  it('preserves the calendar link + a null id (degrade path) so the recorder stays linked', () => {
    // specs/0019 WS6.3 gap b — when the up-front create failed, id is null but the
    // event id/start survive, so the recorder creates a calendar-linked row itself.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({
        id: null,
        title: 'Weekly Sync',
        calendarEventId: 'evt-1',
        startsAt: '2026-06-29T17:00:00Z',
      }),
    );
    expect(consumePendingJoinMeeting()).toEqual({
      id: null,
      title: 'Weekly Sync',
      calendarEventId: 'evt-1',
      startsAt: '2026-06-29T17:00:00Z',
      calendarSeriesKey: null,
    });
  });

  it('peek returns the pending meeting WITHOUT clearing it (double-tap dedupe)', () => {
    // specs/0019 WS6.3 gap c — a second Join & Record on the same event during the
    // pre-record delay must still see the armed event, so it can no-op.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: null, title: 'Standup', calendarEventId: 'evt-9' }),
    );
    expect(peekPendingJoinMeeting()?.calendarEventId).toBe('evt-9');
    // Still there after a peek (unlike consume).
    expect(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY)).not.toBeNull();
    expect(peekPendingJoinMeeting()?.calendarEventId).toBe('evt-9');
  });

  it('returns null when nothing is pending', () => {
    expect(consumePendingJoinMeeting()).toBeNull();
  });

  it('returns null and clears the key for a malformed entry', () => {
    sessionStorage.setItem(PENDING_JOIN_MEETING_KEY, '{ not valid json');
    expect(consumePendingJoinMeeting()).toBeNull();
    expect(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY)).toBeNull();
  });

  it('returns null for a well-formed JSON object missing required fields', () => {
    sessionStorage.setItem(PENDING_JOIN_MEETING_KEY, JSON.stringify({ id: 'only-id' }));
    expect(consumePendingJoinMeeting()).toBeNull();
  });
});

// specs/0041 WS3 — matching a meeting row's stored calendar linkage back to the day-agenda
// item that carries the join link (the detail-page record control's URL source). Pure
// matcher; the agenda fetch itself is exercised by the day-agenda tests.
describe('findAgendaItemForOccurrence', () => {
  const START = '2026-07-08T17:00:00Z';

  function item(overrides: Partial<DayAgendaItem> = {}): DayAgendaItem {
    return {
      id: 'evt-1',
      title: 'Weekly Sync',
      startTime: START,
      endTime: null,
      source: 'calendar',
      zoomUrl: 'https://zoom.us/j/123',
      attendees: [],
      attendeeCount: 0,
      meetingId: null,
      status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
      dismissed: false,
      seriesKey: 'series-1',
      ...overrides,
    };
  }

  const ref = {
    calendarEventId: 'evt-1',
    occurrenceStart: START,
    meetingId: 'meeting-1',
    seriesKey: 'series-1',
  };

  it('prefers the item already linked to this meeting row (recorded occurrence)', () => {
    const linked = item({ id: 'meeting-1', meetingId: 'meeting-1', zoomUrl: 'https://zoom.us/j/999' });
    const other = item({ id: 'evt-1' });
    expect(findAgendaItemForOccurrence([other, linked], ref)?.zoomUrl).toBe(
      'https://zoom.us/j/999',
    );
  });

  it('matches an unrecorded item by its event id (the agenda id IS the event id)', () => {
    expect(findAgendaItemForOccurrence([item()], { ...ref, meetingId: null })).not.toBeNull();
  });

  it('falls back to series key + start time when the event id was reissued', () => {
    // EventKit may reissue `eventIdentifier` between reads; the external id + the
    // occurrence start still pin the item.
    const reissued = item({ id: 'evt-REISSUED' });
    expect(findAgendaItemForOccurrence([reissued], ref)?.id).toBe('evt-REISSUED');
    // …but a different occurrence of the same series (outside the ±30 min window) must not match.
    const otherOccurrence = item({ id: 'evt-REISSUED', startTime: '2026-07-08T09:00:00Z' });
    expect(findAgendaItemForOccurrence([otherOccurrence], ref)).toBeNull();
  });

  it('ignores recording-sourced rows and returns null when nothing matches', () => {
    const recording = item({ source: 'recording', meetingId: 'meeting-1' });
    expect(findAgendaItemForOccurrence([recording], ref)).toBeNull();
    expect(findAgendaItemForOccurrence([], ref)).toBeNull();
  });
});
