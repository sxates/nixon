import { describe, it, expect, beforeEach, vi } from 'vitest';

// specs/0032 task 6 — googleCalendar.ts wrappers are never-throw: while the
// backend commands are being built in parallel (or fail at runtime), every
// helper returns a safe fallback so the Settings card renders instead of
// breaking. Also locks the pinned IPC contract, especially the camelCase
// invoke arg keys for the per-calendar toggle (snake_case keys silently
// deserialize as None on the Rust side).

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT,
  capabilitiesPending,
  capabilityState,
  connectGoogleCalendar,
  disconnectGoogleCalendar,
  formatLastSynced,
  getGoogleCalendarStatus,
  getGoogleCapabilities,
  calendarSyncLine,
  setGoogleCalendarSelected,
  syncChangeCount,
  syncGoogleCalendarNow,
} from '@/lib/googleCalendar';

beforeEach(() => {
  invoke.mockReset();
});

describe('getGoogleCalendarStatus', () => {
  it('returns the backend status as-is (pinned camelCase wire shape)', async () => {
    const status = {
      configured: true,
      connected: true,
      email: 'ada@example.com',
      authRequired: true,
      lastSyncedAt: '2026-07-02T10:00:00Z',
      calendars: [
        {
          id: 'cal-1',
          summary: 'Work',
          selected: true,
          lastSyncedAt: '2026-07-02T10:00:00Z',
          lastError: null,
        },
        {
          id: 'cal-2',
          summary: 'Team',
          selected: true,
          lastSyncedAt: null,
          lastError: 'HTTP 403: forbidden',
        },
      ],
    };
    invoke.mockResolvedValue(status);

    await expect(getGoogleCalendarStatus()).resolves.toEqual(status);
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_status');
  });

  it('never throws: returns the not-configured fallback when the command rejects', async () => {
    invoke.mockRejectedValue(new Error('command api_google_calendar_status not found'));

    await expect(getGoogleCalendarStatus()).resolves.toEqual({
      configured: false,
      connected: false,
      email: null,
      authRequired: false,
      lastSyncedAt: null,
      calendars: [],
    });
  });

  it('normalizes malformed payloads into the safe shape', async () => {
    invoke.mockResolvedValue({ configured: true, calendars: [null, { summary: 'no id' }] });

    await expect(getGoogleCalendarStatus()).resolves.toEqual({
      configured: true,
      connected: false,
      email: null,
      authRequired: false,
      lastSyncedAt: null,
      calendars: [],
    });
  });
});

describe('connectGoogleCalendar', () => {
  it('returns ok + email on success', async () => {
    invoke.mockResolvedValue({ email: 'ada@example.com' });

    await expect(connectGoogleCalendar()).resolves.toEqual({
      ok: true,
      email: 'ada@example.com',
    });
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_connect');
  });

  it('never throws: surfaces the backend’s user-friendly string on failure/cancel', async () => {
    invoke.mockRejectedValue('Connection cancelled — the browser sign-in timed out.');

    await expect(connectGoogleCalendar()).resolves.toEqual({
      ok: false,
      error: 'Connection cancelled — the browser sign-in timed out.',
    });
  });
});

describe('setGoogleCalendarSelected', () => {
  it('sends EXACT camelCase invoke arg keys { calendarId, selected }', async () => {
    invoke.mockResolvedValue(undefined);

    invoke.mockResolvedValue(null);
    await expect(setGoogleCalendarSelected('cal-42', false)).resolves.toEqual({
      ok: true,
      outcome: null,
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_set_calendar_selected', {
      calendarId: 'cal-42',
      selected: false,
    });
    // Belt-and-braces: no snake_case key ever crosses IPC (it would silently
    // deserialize as None/default on the Rust side).
    const args = invoke.mock.calls[0][1] as Record<string, unknown>;
    expect(Object.keys(args)).toEqual(['calendarId', 'selected']);
  });

  it('never throws: returns ok:false when the command rejects', async () => {
    invoke.mockRejectedValue(new Error('boom'));
    await expect(setGoogleCalendarSelected('cal-42', true)).resolves.toEqual({ ok: false });
  });

  it('passes an enabling sync outcome through, per-calendar error included', async () => {
    invoke.mockResolvedValue({
      kind: 'synced',
      durationMs: 40,
      calendars: [{ calendarId: 'cal-42', summary: 'Work', error: 'HTTP 500' }],
    });
    const result = await setGoogleCalendarSelected('cal-42', true);
    expect(result.ok).toBe(true);
    const outcome = result.ok ? result.outcome : null;
    expect(outcome?.kind).toBe('synced');
    expect(outcome?.kind === 'synced' && outcome.calendars[0].error).toBe('HTTP 500');
  });
});

describe('disconnect / sync now', () => {
  it('disconnectGoogleCalendar returns true on success, false on failure', async () => {
    invoke.mockResolvedValue(undefined);
    await expect(disconnectGoogleCalendar()).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_disconnect');

    invoke.mockRejectedValue(new Error('boom'));
    await expect(disconnectGoogleCalendar()).resolves.toBe(false);
  });

  it('syncGoogleCalendarNow returns the parsed outcome, not a boolean', async () => {
    invoke.mockResolvedValue({ kind: 'alreadyRunning' });
    await expect(syncGoogleCalendarNow()).resolves.toEqual({ kind: 'alreadyRunning' });
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_sync_now');

    invoke.mockResolvedValue({
      kind: 'synced',
      durationMs: 812,
      calendars: [
        {
          calendarId: 'primary',
          summary: 'Work',
          isPrimary: true,
          mode: 'incremental',
          fetched: 3,
          upserted: 2,
          deleted: 1,
          durationMs: 800,
          error: null,
        },
      ],
    });
    const outcome = await syncGoogleCalendarNow();
    expect(outcome.kind).toBe('synced');
    expect(syncChangeCount(outcome)).toBe(3);
  });

  it('syncGoogleCalendarNow throws the backend message when the pass fails', async () => {
    invoke.mockRejectedValue('Google Calendar sync failed: timed out after 120s');
    await expect(syncGoogleCalendarNow()).rejects.toThrow(
      'Google Calendar sync failed: timed out after 120s',
    );
  });

  it('syncGoogleCalendarNow refuses an unrecognized payload instead of calling it a sync', async () => {
    invoke.mockResolvedValue(undefined);
    await expect(syncGoogleCalendarNow()).rejects.toThrow(/unexpected result/);
    invoke.mockResolvedValue({ kind: 'mystery' });
    await expect(syncGoogleCalendarNow()).rejects.toThrow(/unexpected result/);
  });
});

describe('calendarSyncLine', () => {
  const now = new Date('2026-07-02T12:00:00Z');
  const base = { id: 'c', summary: 'Work', selected: true };

  it('says when a selected calendar last synced', () => {
    expect(calendarSyncLine({ ...base, lastSyncedAt: '2026-07-02T11:57:00Z' }, now)).toEqual({
      text: 'synced 3 min ago',
      failed: false,
    });
    expect(calendarSyncLine({ ...base, lastSyncedAt: '2026-07-02T11:59:40Z' }, now)?.text).toBe(
      'synced just now',
    );
    expect(calendarSyncLine({ ...base, lastSyncedAt: '2026-07-02T09:00:00Z' }, now)?.text).toBe(
      'synced 3 h ago',
    );
    expect(calendarSyncLine({ ...base, lastSyncedAt: null }, now)?.text).toBe('not synced yet');
  });

  it('a failure wins over the last good sync time', () => {
    expect(
      calendarSyncLine(
        { ...base, lastSyncedAt: '2026-07-02T11:57:00Z', lastError: 'HTTP 403' },
        now,
      ),
    ).toEqual({ text: 'failed: HTTP 403', failed: true });
  });

  it('says nothing for a deselected calendar', () => {
    expect(calendarSyncLine({ ...base, selected: false, lastError: 'x' }, now)).toBeNull();
  });
});

describe('getGoogleCapabilities (specs/0038 WS3)', () => {
  it('passes through the pinned tri-state wire shape', async () => {
    invoke.mockResolvedValue({
      canExpandGroups: true,
      canFetchPhotos: false,
      probedAt: '2026-07-07T10:00:00Z',
    });

    await expect(getGoogleCapabilities()).resolves.toEqual({
      canExpandGroups: true,
      canFetchPhotos: false,
      probedAt: '2026-07-07T10:00:00Z',
    });
    expect(invoke).toHaveBeenCalledWith('api_google_capabilities');
  });

  it('normalizes an unprobed/just-connected payload (nulls) as-is', async () => {
    invoke.mockResolvedValue({
      canExpandGroups: null,
      canFetchPhotos: null,
      probedAt: null,
    });
    await expect(getGoogleCapabilities()).resolves.toEqual({
      canExpandGroups: null,
      canFetchPhotos: null,
      probedAt: null,
    });
  });

  it('never throws: returns all-null when the command is missing/rejects', async () => {
    invoke.mockRejectedValue(new Error('command api_google_capabilities not found'));
    await expect(getGoogleCapabilities()).resolves.toEqual({
      canExpandGroups: null,
      canFetchPhotos: null,
      probedAt: null,
    });
  });

  it('coerces a malformed payload into the safe tri-state', async () => {
    invoke.mockResolvedValue({ canExpandGroups: 'yes', probedAt: 42 });
    await expect(getGoogleCapabilities()).resolves.toEqual({
      canExpandGroups: null,
      canFetchPhotos: null,
      probedAt: null,
    });
  });
});

describe('capability tri-state helpers', () => {
  it('capabilityState maps true→on, false→off, null→checking', () => {
    expect(capabilityState(true)).toBe('on');
    expect(capabilityState(false)).toBe('off');
    expect(capabilityState(null)).toBe('checking');
  });

  it('capabilitiesPending is true while any flag is unprobed', () => {
    expect(capabilitiesPending({ canExpandGroups: null, canFetchPhotos: true, probedAt: null })).toBe(
      true,
    );
    expect(
      capabilitiesPending({ canExpandGroups: true, canFetchPhotos: null, probedAt: null }),
    ).toBe(true);
    expect(
      capabilitiesPending({
        canExpandGroups: true,
        canFetchPhotos: false,
        probedAt: '2026-07-07T10:00:00Z',
      }),
    ).toBe(false);
  });
});

describe('event name + last-synced formatting', () => {
  it('exposes the pinned Rust→frontend event name', () => {
    expect(GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT).toBe('google-calendar-auth-required');
  });

  it('formats relative last-synced times', () => {
    const now = new Date('2026-07-02T12:00:00Z');
    expect(formatLastSynced(null, now)).toBe('Not synced yet');
    expect(formatLastSynced('garbage', now)).toBe('Not synced yet');
    expect(formatLastSynced('2026-07-02T11:59:40Z', now)).toBe('Last synced just now');
    expect(formatLastSynced('2026-07-02T11:35:00Z', now)).toBe('Last synced 25m ago');
    expect(formatLastSynced('2026-07-02T09:00:00Z', now)).toBe('Last synced 3h ago');
    expect(formatLastSynced('2026-06-30T09:00:00Z', now)).toBe('Last synced 2d ago');
  });
});
