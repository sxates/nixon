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
  setGoogleCalendarSelected,
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
      lastSyncedAt: '2026-07-02T10:00:00Z',
      calendars: [{ id: 'cal-1', summary: 'Work', selected: true }],
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

    await expect(setGoogleCalendarSelected('cal-42', false)).resolves.toBe(true);
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

  it('never throws: returns false when the command rejects', async () => {
    invoke.mockRejectedValue(new Error('boom'));
    await expect(setGoogleCalendarSelected('cal-42', true)).resolves.toBe(false);
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

  it('syncGoogleCalendarNow returns true on success, false on failure', async () => {
    invoke.mockResolvedValue(undefined);
    await expect(syncGoogleCalendarNow()).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('api_google_calendar_sync_now');

    invoke.mockRejectedValue(new Error('boom'));
    await expect(syncGoogleCalendarNow()).resolves.toBe(false);
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
