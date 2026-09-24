import { describe, it, expect, beforeEach } from 'vitest';
import {
  agendaPhase,
  canJoinAgendaItem,
  initials,
  readCachedAgenda,
  cacheAgenda,
  clearCachedAgenda,
  setItemDismissed,
  dismissKeysFor,
  type DayAgendaItem,
} from '@/lib/day-agenda';

// specs/0023 L1/L3 — pure-logic example: no DOM, no IPC. Proves the harness runs
// and locks the agenda time-window classification the home screen depends on.

function item(partial: Partial<DayAgendaItem>): DayAgendaItem {
  return {
    id: 'x',
    title: 'Standup',
    startTime: '2026-06-29T10:00:00Z',
    endTime: '2026-06-29T10:30:00Z',
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: null,
    status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    ...partial,
  };
}

describe('agendaPhase', () => {
  const now = new Date('2026-06-29T10:15:00Z');

  it('classifies an event that has started and not ended as "now"', () => {
    expect(agendaPhase(item({}), now)).toBe('now');
  });

  it('classifies a future event as "upcoming"', () => {
    expect(
      agendaPhase(item({ startTime: '2026-06-29T11:00:00Z', endTime: '2026-06-29T11:30:00Z' }), now),
    ).toBe('upcoming');
  });

  it('opens "now" 5 minutes before the official start (pre-start grace)', () => {
    // now = 10:15Z; a start 4 min out (10:19Z) is inside the 5-min grace → "now".
    expect(
      agendaPhase(item({ startTime: '2026-06-29T10:19:00Z', endTime: '2026-06-29T10:49:00Z' }), now),
    ).toBe('now');
  });

  it('stays "upcoming" more than 5 minutes before the start', () => {
    // now = 10:15Z; a start 6 min out (10:21Z) is outside the grace → still "upcoming".
    expect(
      agendaPhase(item({ startTime: '2026-06-29T10:21:00Z', endTime: '2026-06-29T10:51:00Z' }), now),
    ).toBe('upcoming');
  });

  it('classifies an ended event as "past"', () => {
    expect(
      agendaPhase(item({ startTime: '2026-06-29T09:00:00Z', endTime: '2026-06-29T09:30:00Z' }), now),
    ).toBe('past');
  });

  it('treats a no-end event as "now" inside the 90-minute grace window', () => {
    expect(
      agendaPhase(item({ startTime: '2026-06-29T09:30:00Z', endTime: null }), now),
    ).toBe('now');
  });

  it('treats a no-end event as "past" once the grace window elapses', () => {
    expect(
      agendaPhase(item({ startTime: '2026-06-29T08:00:00Z', endTime: null }), now),
    ).toBe('past');
  });

  it('falls back to "upcoming" for an unparseable start time', () => {
    expect(agendaPhase(item({ startTime: 'not-a-date' }), now)).toBe('upcoming');
  });
});

describe('canJoinAgendaItem (WS6.4)', () => {
  const base = {
    hasZoomUrl: true,
    recorded: false,
    isRecordingThis: false,
    anyRecordingInProgress: false,
  };

  it('offers Join & Record for a joinable, not-yet-recorded row when idle', () => {
    expect(canJoinAgendaItem(base)).toBe(true);
  });

  it('does not offer it while ANY recording is in progress (different event)', () => {
    expect(canJoinAgendaItem({ ...base, anyRecordingInProgress: true })).toBe(false);
  });

  it('does not offer it on the row currently being recorded', () => {
    expect(canJoinAgendaItem({ ...base, isRecordingThis: true })).toBe(false);
  });

  it('does not offer it once recorded, or without a join link', () => {
    expect(canJoinAgendaItem({ ...base, recorded: true })).toBe(false);
    expect(canJoinAgendaItem({ ...base, hasZoomUrl: false })).toBe(false);
  });
});

describe('initials', () => {
  it('takes first+last initials of a full name', () => {
    expect(initials('Ada Example')).toBe('AE');
  });

  it('uses the email local part', () => {
    expect(initials('jordan.lee@example.com')).toBe('JL');
  });

  it('handles a single name', () => {
    expect(initials('Madonna')).toBe('MA');
  });

  it('returns "?" for empty input', () => {
    expect(initials('   ')).toBe('?');
  });
});

// specs/0024 WS5.1 — last-good agenda cache bridges transient empty/cold EventKit reads.
describe('agenda last-good cache', () => {
  beforeEach(() => sessionStorage.clear());

  const DAY = '2026-09-23';

  it('round-trips a calendar-bearing agenda', () => {
    const agenda = [item({ id: 'a', source: 'calendar' })];
    cacheAgenda(DAY, agenda);
    expect(readCachedAgenda(DAY)).toEqual(agenda);
  });

  it('does NOT cache a recordings-only read (so a cold read cannot clobber a good cache)', () => {
    cacheAgenda(DAY, [item({ id: 'good', source: 'calendar' })]);
    cacheAgenda(DAY, [item({ id: 'rec', source: 'recording' })]); // should be ignored
    expect(readCachedAgenda(DAY)).toEqual([item({ id: 'good', source: 'calendar' })]);
  });

  it('returns [] when nothing is cached', () => {
    expect(readCachedAgenda(DAY)).toEqual([]);
  });

  // specs/0075 W3 — the cache used to be one key, so a fresh mount could show the rows
  // of whatever day was cached last.
  it("never returns one date's rows for another date", () => {
    cacheAgenda('2026-09-22', [item({ id: 'yesterday', source: 'calendar' })]);
    expect(readCachedAgenda(DAY)).toEqual([]);
    expect(readCachedAgenda('2026-09-22')).toHaveLength(1);
  });

  it('clearCachedAgenda forgets only that date', () => {
    cacheAgenda(DAY, [item({ id: 'a', source: 'calendar' })]);
    cacheAgenda('2026-09-22', [item({ id: 'b', source: 'calendar' })]);
    clearCachedAgenda(DAY);
    expect(readCachedAgenda(DAY)).toEqual([]);
    expect(readCachedAgenda('2026-09-22')).toHaveLength(1);
  });
});

// specs/0029 WS6.2 — optimistic hide/unhide state transition.
describe('setItemDismissed (optimistic hide/unhide)', () => {
  const agenda = [item({ id: 'a' }), item({ id: 'b' })];

  it('flips only the matching item to dismissed (hide)', () => {
    const next = setItemDismissed(agenda, 'a', true);
    expect(next.find((it) => it.id === 'a')?.dismissed).toBe(true);
    expect(next.find((it) => it.id === 'b')?.dismissed).toBe(false);
  });

  it('flips back on revert/unhide (round-trips)', () => {
    const hidden = setItemDismissed(agenda, 'a', true);
    expect(setItemDismissed(hidden, 'a', false)).toEqual(agenda);
  });

  it('does not mutate the input array', () => {
    setItemDismissed(agenda, 'a', true);
    expect(agenda.find((it) => it.id === 'a')?.dismissed).toBe(false);
  });

  it('is a no-op for an unknown id (a revert can never invent rows)', () => {
    expect(setItemDismissed(agenda, 'nope', true)).toEqual(agenda);
  });
});

// specs/0029 WS6.2 — dual-key dismissal (write stable key, clear both on unhide).
describe('dismissKeysFor', () => {
  it('puts the stable dismissKey first, legacy id second', () => {
    expect(dismissKeysFor({ id: 'ek-1', dismissKey: 'ext:EXT@2026-06-29T10:00:00Z' })).toEqual([
      'ext:EXT@2026-06-29T10:00:00Z',
      'ek-1',
    ]);
  });

  it('dedupes when the backend fell back to the row id', () => {
    expect(dismissKeysFor({ id: 'ek-1', dismissKey: 'ek-1' })).toEqual(['ek-1']);
  });

  it('falls back to the row id for stale cached items without a dismissKey', () => {
    expect(dismissKeysFor({ id: 'ek-1' })).toEqual(['ek-1']);
  });
});
