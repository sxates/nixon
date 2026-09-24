import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

// specs/0032 task 6 — the Settings → Calendar card renders four Google states
// driven by `api_google_calendar_status` (not configured / disconnected /
// connecting / connected), sends camelCase toggle args, confirms disconnect,
// and shows toast + banner on the `google-calendar-auth-required` event.
//
// Single active source (no merged view): `connected: true` means Google is the
// active source. The card shows an "Active" badge on the macOS row while Google
// is disconnected, and on the Google row (with a de-emphasized macOS row) while
// connected.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const { listeners, listenMock } = vi.hoisted(() => {
  const listeners: Record<string, (event: unknown) => void> = {};
  return {
    listeners,
    listenMock: vi.fn((event: string, cb: (e: unknown) => void) => {
      listeners[event] = cb;
      return Promise.resolve(() => {
        delete listeners[event];
      });
    }),
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

const { toastMock } = vi.hoisted(() => ({
  toastMock: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() },
}));
vi.mock('sonner', () => ({ toast: toastMock }));


import { CalendarSettings } from '../CalendarSettings';
import type { GoogleCalendarStatus } from '@/lib/googleCalendar';

const PRIVACY_COPY =
  'Read-only. Nixon downloads event details (titles, times, attendees) to this Mac. ' +
  'Your recordings, transcripts, and notes are never uploaded — to Google or anyone.';

const CONNECTED_STATUS: GoogleCalendarStatus = {
  configured: true,
  connected: true,
  email: 'ada@example.com',
  lastSyncedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
  calendars: [
    { id: 'cal-1', summary: 'Work', selected: true },
    { id: 'cal-2', summary: 'Family', selected: false },
  ],
};

/** Route mocked invokes by command; EventKit reads as authorized by default. */
function mockBackend(
  status: Partial<GoogleCalendarStatus>,
  overrides: Record<string, (args?: unknown) => unknown> = {},
) {
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in overrides) return overrides[cmd](args);
    switch (cmd) {
      case 'api_get_calendar_access_status':
        return 'authorized';
      case 'api_google_calendar_status':
        return {
          configured: false,
          connected: false,
          email: null,
          lastSyncedAt: null,
          calendars: [],
          ...status,
        };
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  invoke.mockReset();
  listenMock.mockClear();
  toastMock.success.mockReset();
  toastMock.error.mockReset();
  toastMock.warning.mockReset();
  toastMock.info.mockReset();
  for (const key of Object.keys(listeners)) delete listeners[key];
});

/** The two source rows, scoped for badge assertions. */
function macosRow() {
  return screen.getByRole('group', { name: 'macOS Calendar source' });
}
function googleRow() {
  return screen.getByRole('group', { name: 'Google Calendar source' });
}

describe('CalendarSettings — Google row render states', () => {
  it('(a) not configured: muted copy, no Connect button, macOS row is the Active source', async () => {
    mockBackend({ configured: false });
    render(<CalendarSettings />);

    expect(
      await screen.findByText("Google Calendar isn't configured in this build."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /connect google calendar/i }),
    ).toBeNull();
    // Single-source lead-in under the card heading.
    expect(
      screen.getByText(
        "Nixon uses one calendar source — your Mac's calendar, or Google Calendar connected directly.",
      ),
    ).toBeInTheDocument();
    // The macOS row survives the relocation (authorized → no Connect button)
    // and carries the sole Active badge while Google is not connected.
    expect(screen.getByText('macOS Calendar')).toBeInTheDocument();
    expect(within(macosRow()).getByText('Active')).toBeInTheDocument();
    expect(screen.getAllByText('Active')).toHaveLength(1);
  });

  it('(b) disconnected: Connect button + source-switch note + verbatim ADR-0010 privacy copy + unverified-app note; Active badge on the macOS row', async () => {
    mockBackend({ configured: true, connected: false });
    render(<CalendarSettings />);

    expect(
      await screen.findByRole('button', { name: /connect google calendar/i }),
    ).toBeEnabled();
    expect(
      screen.getByText("Connecting switches Nixon's calendar source to Google Calendar."),
    ).toBeInTheDocument();
    expect(screen.getByText(PRIVACY_COPY)).toBeInTheDocument();
    expect(screen.getByText(/unverified app/)).toBeInTheDocument();
    expect(screen.getByText(/Advanced → Continue/)).toBeInTheDocument();
    // macOS Calendar is the active source; the Google row is not.
    expect(within(macosRow()).getByText('Active')).toBeInTheDocument();
    expect(within(googleRow()).queryByText('Active')).toBeNull();
  });

  it('(c) connecting: pending browser-consent copy with a cancel affordance', async () => {
    mockBackend(
      { configured: true, connected: false },
      { api_google_calendar_connect: () => new Promise(() => {}) }, // never settles
    );
    render(<CalendarSettings />);

    fireEvent.click(
      await screen.findByRole('button', { name: /connect google calendar/i }),
    );

    expect(
      await screen.findByText('Complete the connection in your browser…'),
    ).toBeInTheDocument();
    const cancel = screen.getByRole('button', { name: /^cancel$/i });

    // Dismissing returns to the disconnected state (the backend invoke itself
    // times out after 5 minutes; the UI just stops waiting).
    fireEvent.click(cancel);
    expect(
      await screen.findByRole('button', { name: /connect google calendar/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText('Complete the connection in your browser…'),
    ).toBeNull();
  });

  // specs/0061 W1 Task 3 fix (controller ruling R14) — the gap that hid the
  // original defect: the test above uses a connect promise that never
  // resolves, so a dismissed-then-resolved connect was never exercised. The
  // backend invoke itself can't be aborted (it waits up to 5 minutes for
  // browser consent), so after Cancel it can still resolve — and it must do
  // so silently: no toast (sonner renders at the app root, so it would
  // surface even after navigating away from Settings), and no extra status
  // re-fetch.
  it('(c) dismissing, then letting the connect call resolve anyway, stays silent (no toast, no re-fetch)', async () => {
    let resolveConnect: (value: { email: string }) => void = () => {};
    mockBackend(
      { configured: true, connected: false },
      {
        api_google_calendar_connect: () =>
          new Promise((resolve) => {
            resolveConnect = resolve;
          }),
      },
    );
    render(<CalendarSettings />);

    fireEvent.click(
      await screen.findByRole('button', { name: /connect google calendar/i }),
    );
    const cancel = await screen.findByRole('button', { name: /^cancel$/i });

    const statusCallsBeforeDismiss = invoke.mock.calls.filter(
      ([cmd]) => cmd === 'api_google_calendar_status',
    ).length;

    fireEvent.click(cancel);
    await screen.findByRole('button', { name: /connect google calendar/i });

    // The backend call resolves anyway, well after the dismiss.
    await act(async () => {
      resolveConnect({ email: 'ada@example.com' });
      // Flush every microtask in the hook's post-resolution chain.
      await new Promise((r) => setTimeout(r, 0));
    });

    expect(toastMock.success).not.toHaveBeenCalled();
    expect(toastMock.error).not.toHaveBeenCalled();
    const statusCallsAfter = invoke.mock.calls.filter(
      ([cmd]) => cmd === 'api_google_calendar_status',
    ).length;
    expect(statusCallsAfter).toBe(statusCallsBeforeDismiss);
    // Still shows the disconnected state — the dismissed connect did not
    // silently flip the card to "connected".
    expect(
      await screen.findByRole('button', { name: /connect google calendar/i }),
    ).toBeInTheDocument();
  });

  it('(d) connected: email, last-synced, per-calendar checkboxes, Sync now, Disconnect', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    expect(
      await screen.findByText(/Connected as ada@example\.com/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Last synced 5m ago/)).toBeInTheDocument();

    expect(screen.getByRole('checkbox', { name: 'Work' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Family' })).not.toBeChecked();

    expect(screen.getByRole('button', { name: /sync now/i })).toBeEnabled();
    expect(screen.getByRole('button', { name: /^disconnect$/i })).toBeEnabled();
  });

  it('(d) connected: Google row is the Active source; macOS row is de-emphasized', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    await screen.findByText(/Connected as ada@example\.com/);

    // Exactly one Active badge, and it's on the Google row.
    expect(within(googleRow()).getByText('Active')).toBeInTheDocument();
    expect(within(macosRow()).queryByText('Active')).toBeNull();
    expect(screen.getAllByText('Active')).toHaveLength(1);

    // The macOS row is muted while Google is the source (permission status
    // stays visible, de-emphasized).
    expect(
      within(macosRow()).getByText(/Not used while Google Calendar is connected/),
    ).toBeInTheDocument();
  });
});

describe('CalendarSettings — connected-state actions', () => {
  it('toggling a calendar sends EXACT camelCase invoke args { calendarId, selected }', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('checkbox', { name: 'Work' }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_set_calendar_selected', {
        calendarId: 'cal-1',
        selected: false,
      }),
    );
    const call = invoke.mock.calls.find(
      ([cmd]) => cmd === 'api_google_calendar_set_calendar_selected',
    );
    expect(Object.keys(call![1] as Record<string, unknown>)).toEqual([
      'calendarId',
      'selected',
    ]);
    // Optimistic UI: the checkbox flipped immediately.
    expect(screen.getByRole('checkbox', { name: 'Work' })).not.toBeChecked();
  });

  it('reverts the optimistic toggle and toasts when the command fails', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_calendar_set_calendar_selected: () => Promise.reject(new Error('boom')),
    });
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('checkbox', { name: 'Work' }));

    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'Work' })).toBeChecked(),
    );
    expect(toastMock.error).toHaveBeenCalledWith('Could not update calendar selection');
  });

  // specs/0041 WS5 — bulk Select all / none above the checkbox list, batched
  // through the single api_google_calendar_set_calendars_selected command.
  it('"Select all" batches only the calendars changing state and checks all boxes optimistically', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('button', { name: /select all/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_set_calendars_selected', {
        calendarIds: ['cal-2'], // 'cal-1' was already selected — not re-sent
        selected: true,
      }),
    );
    expect(screen.getByRole('checkbox', { name: 'Work' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Family' })).toBeChecked();
  });

  it('"Select none" unchecks every calendar through the bulk command', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('button', { name: /select none/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_set_calendars_selected', {
        calendarIds: ['cal-1'], // 'cal-2' was already off — not re-sent
        selected: false,
      }),
    );
    expect(screen.getByRole('checkbox', { name: 'Work' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Family' })).not.toBeChecked();
  });

  it('reverts the optimistic bulk toggle and toasts when the command fails', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_calendar_set_calendars_selected: () => Promise.reject(new Error('boom')),
    });
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('button', { name: /select none/i }));

    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'Work' })).toBeChecked(),
    );
    expect(screen.getByRole('checkbox', { name: 'Family' })).not.toBeChecked();
    expect(toastMock.error).toHaveBeenCalledWith('Could not update calendar selection');
  });

  it('"Sync now" invokes api_google_calendar_sync_now and refreshes status', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_calendar_sync_now: () => ({ kind: 'synced', calendars: [], durationMs: 5 }),
    });
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('button', { name: /sync now/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_sync_now'),
    );
    await waitFor(() =>
      expect(toastMock.success).toHaveBeenCalledWith(
        'Google Calendar synced — no changes',
        expect.anything(),
      ),
    );
    const statusReads = invoke.mock.calls.filter((c) => c[0] === 'api_google_calendar_status');
    expect(statusReads.length).toBeGreaterThanOrEqual(2);
  });

  it('Disconnect opens a confirm dialog (back-to-Mac-calendar copy) and only disconnects on confirm', async () => {
    mockBackend(CONNECTED_STATUS);
    render(<CalendarSettings />);

    fireEvent.click(await screen.findByRole('button', { name: /^disconnect$/i }));

    // Confirm dialog: switching the source back + local-copy deletion.
    expect(
      await screen.findByText(/go back to using your Mac's calendar/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/local copy of your Google events is deleted/i),
    ).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('api_google_calendar_disconnect');

    fireEvent.click(
      screen.getByRole('button', { name: /^disconnect google calendar$/i }),
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_disconnect'),
    );
    await waitFor(() => expect(toastMock.success).toHaveBeenCalled());
  });
});

describe('CalendarSettings — enhanced attendee details (specs/0038 WS3)', () => {
  it('shows honest per-account capability statuses when connected + probed', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_capabilities: () => ({
        canExpandGroups: true,
        canFetchPhotos: false,
        probedAt: '2026-07-07T10:00:00Z',
      }),
    });
    render(<CalendarSettings />);

    expect(await screen.findByText('Enhanced attendee details')).toBeInTheDocument();
    // DL expansion allowed by the org; photos denied by policy.
    expect(await screen.findByText('expanding to members')).toBeInTheDocument();
    expect(screen.getByText('not available')).toBeInTheDocument();
    // Honest helper line about org policy gating.
    expect(
      screen.getByText(/Availability depends on your organization/i),
    ).toBeInTheDocument();
  });

  it('shows the group-restriction note when expansion is off, and hides it when on/checking (RC-2)', async () => {
    const noteRe = /Your organization restricts Google group access/i;

    // off: canExpandGroups=false → the explanatory note appears.
    mockBackend(CONNECTED_STATUS, {
      api_google_capabilities: () => ({
        canExpandGroups: false,
        canFetchPhotos: true,
        probedAt: '2026-07-07T10:00:00Z',
      }),
    });
    const { unmount } = render(<CalendarSettings />);
    expect(await screen.findByText(noteRe)).toBeInTheDocument();
    unmount();

    // on: canExpandGroups=true → no note.
    mockBackend(CONNECTED_STATUS, {
      api_google_capabilities: () => ({
        canExpandGroups: true,
        canFetchPhotos: true,
        probedAt: '2026-07-07T10:00:00Z',
      }),
    });
    const on = render(<CalendarSettings />);
    await screen.findByText('Enhanced attendee details');
    expect(screen.queryByText(noteRe)).toBeNull();
    on.unmount();

    // checking (null) → no note.
    mockBackend(CONNECTED_STATUS, {
      api_google_capabilities: () => ({
        canExpandGroups: null,
        canFetchPhotos: null,
        probedAt: null,
      }),
    });
    render(<CalendarSettings />);
    await screen.findByText('Enhanced attendee details');
    expect(screen.queryByText(noteRe)).toBeNull();
  });

  it('renders "checking…" for both while the fire-and-forget probe is still running', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_capabilities: () => ({
        canExpandGroups: null,
        canFetchPhotos: null,
        probedAt: null,
      }),
    });
    render(<CalendarSettings />);

    await screen.findByText('Enhanced attendee details');
    expect(screen.getAllByText('checking…')).toHaveLength(2);
  });

  it('does not query capabilities while Google is disconnected', async () => {
    mockBackend({ configured: true, connected: false });
    render(<CalendarSettings />);

    await screen.findByRole('button', { name: /connect google calendar/i });
    expect(screen.queryByText('Enhanced attendee details')).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith('api_google_capabilities');
  });
});

describe('CalendarSettings — google-calendar-auth-required event', () => {
  it('shows a toast and a persistent reconnect banner, and Reconnect re-runs the connect flow', async () => {
    mockBackend(
      { configured: true, connected: true, email: 'ada@example.com', calendars: [] },
      { api_google_calendar_connect: () => new Promise(() => {}) },
    );
    render(<CalendarSettings />);

    // Wait for the listener registration (async .then in the effect).
    await waitFor(() =>
      expect(listeners['google-calendar-auth-required']).toBeDefined(),
    );

    act(() => {
      listeners['google-calendar-auth-required']({ payload: null });
    });

    expect(toastMock.error).toHaveBeenCalledWith(
      'Google needs you to reconnect',
      expect.objectContaining({ description: expect.stringMatching(/reconnect/i) }),
    );
    expect(
      await screen.findByText('Google Calendar needs reconnecting'),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^reconnect$/i }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_google_calendar_connect'),
    );
  });
});

// ---------------------------------------------------------------------------
// specs/0074 W2 / 0075 W1b — Sync now tells the truth, per outcome.
// ---------------------------------------------------------------------------

function report(summary: string, over: Record<string, unknown> = {}) {
  return {
    calendarId: summary.toLowerCase(),
    summary,
    isPrimary: false,
    mode: 'incremental',
    fetched: 0,
    upserted: 0,
    deleted: 0,
    durationMs: 10,
    error: null,
    ...over,
  };
}

async function clickSyncNow(outcome: unknown, reject = false) {
  mockBackend(CONNECTED_STATUS, {
    api_google_calendar_sync_now: () => (reject ? Promise.reject(outcome) : outcome),
    api_google_calendar_connect: () => new Promise(() => {}),
  });
  render(<CalendarSettings />);
  fireEvent.click(await screen.findByRole('button', { name: /sync now/i }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_google_calendar_sync_now'));
  // Let the handler settle (the button returns to "Sync now").
  await screen.findByRole('button', { name: /^sync now$/i });
}

/** Every title any toast channel was called with — to prove what was NOT said. */
function allToastTitles(): string[] {
  return [toastMock.success, toastMock.error, toastMock.warning, toastMock.info].flatMap((m) =>
    m.mock.calls.map((c) => String(c[0])),
  );
}

describe('CalendarSettings — Sync now toasts by outcome', () => {
  it('synced, clean: "Google Calendar synced — N changes"', async () => {
    await clickSyncNow({
      kind: 'synced',
      durationMs: 30,
      calendars: [report('Work', { upserted: 2 }), report('Team', { deleted: 1 })],
    });
    expect(toastMock.success).toHaveBeenCalledWith(
      'Google Calendar synced — 3 changes',
      expect.anything(),
    );
  });

  it('synced with a calendar error: a warning naming that calendar, never green', async () => {
    await clickSyncNow({
      kind: 'synced',
      durationMs: 30,
      calendars: [report('Work', { upserted: 2 }), report('Team', { error: 'HTTP 403: forbidden' })],
    });
    expect(toastMock.warning).toHaveBeenCalledWith(
      'Synced, but Team failed',
      expect.objectContaining({ description: 'HTTP 403: forbidden' }),
    );
    expect(toastMock.success).not.toHaveBeenCalled();
  });

  it('authRequired: error toast with Reconnect, sets the banner, never says "synced"', async () => {
    await clickSyncNow({ kind: 'authRequired' });
    expect(toastMock.error).toHaveBeenCalledWith(
      'Google needs you to reconnect',
      expect.objectContaining({ action: expect.objectContaining({ label: 'Reconnect' }) }),
    );
    expect(toastMock.success).not.toHaveBeenCalled();
    expect(allToastTitles().some((t) => /synced/i.test(t))).toBe(false);
    expect(await screen.findByText('Google Calendar needs reconnecting')).toBeInTheDocument();

    // The toast's Reconnect action runs the existing connect flow.
    const opts = toastMock.error.mock.calls[0][1] as { action: { onClick: () => void } };
    act(() => opts.action.onClick());
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_google_calendar_connect'));
  });

  it('alreadyRunning: "A sync is still running", never says "synced"', async () => {
    await clickSyncNow({ kind: 'alreadyRunning' });
    expect(toastMock.info).toHaveBeenCalledWith('A sync is still running', expect.anything());
    expect(toastMock.success).not.toHaveBeenCalled();
    expect(allToastTitles().some((t) => /\bsynced\b/i.test(t) && !/nothing/i.test(t))).toBe(false);
  });

  it.each([
    ['notConnected', /Google Calendar isn't connected/],
    ['noCalendarsSelected', /no calendars are selected/],
    ['suppressed', /demo data is loaded/],
    ['notConfigured', /isn't set up in this build/],
  ])('%s: a neutral toast naming the reason, never green', async (kind, title) => {
    await clickSyncNow({ kind });
    expect(toastMock.info).toHaveBeenCalledWith(expect.stringMatching(title), expect.anything());
    expect(toastMock.success).not.toHaveBeenCalled();
    expect(toastMock.error).not.toHaveBeenCalled();
  });

  it('a failed pass shows the backend reason', async () => {
    await clickSyncNow('Google Calendar sync failed: timed out after 120s', true);
    expect(toastMock.error).toHaveBeenCalledWith(
      'Sync failed',
      expect.objectContaining({ description: 'Google Calendar sync failed: timed out after 120s' }),
    );
    expect(toastMock.success).not.toHaveBeenCalled();
  });
});

describe('CalendarSettings — latched auth + per-calendar status', () => {
  it('shows the reconnect banner on load when the status reports authRequired (no event needed)', async () => {
    mockBackend({ ...CONNECTED_STATUS, authRequired: true });
    render(<CalendarSettings />);
    expect(await screen.findByText('Google Calendar needs reconnecting')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^reconnect$/i })).toBeInTheDocument();
  });

  it('shows no banner when the status is not latched', async () => {
    mockBackend({ ...CONNECTED_STATUS, authRequired: false });
    render(<CalendarSettings />);
    await screen.findByText(/Connected as ada@example\.com/);
    expect(screen.queryByText('Google Calendar needs reconnecting')).toBeNull();
  });

  it('each selected calendar shows when it synced, or why it failed', async () => {
    mockBackend({
      ...CONNECTED_STATUS,
      calendars: [
        {
          id: 'cal-1',
          summary: 'Work',
          selected: true,
          lastSyncedAt: new Date(Date.now() - 3 * 60_000).toISOString(),
          lastError: null,
        },
        { id: 'cal-3', summary: 'Team', selected: true, lastSyncedAt: null, lastError: 'HTTP 403' },
        { id: 'cal-2', summary: 'Family', selected: false, lastSyncedAt: null, lastError: null },
      ],
    });
    render(<CalendarSettings />);
    expect(await screen.findByTestId('calendar-sync-line-cal-1')).toHaveTextContent(
      'synced 3 min ago',
    );
    const failed = screen.getByTestId('calendar-sync-line-cal-3');
    expect(failed).toHaveTextContent('failed: HTTP 403');
    expect(failed.className).toContain('text-destructive');
    expect(screen.queryByTestId('calendar-sync-line-cal-2')).toBeNull();
  });

  it('ticking a calendar whose first sync fails warns, and re-reads the status', async () => {
    mockBackend(CONNECTED_STATUS, {
      api_google_calendar_set_calendar_selected: () => ({
        kind: 'synced',
        durationMs: 9,
        calendars: [report('Family', { error: 'HTTP 404: notFound' })],
      }),
    });
    render(<CalendarSettings />);
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Family' }));
    await waitFor(() =>
      expect(toastMock.warning).toHaveBeenCalledWith(
        'Synced, but Family failed',
        expect.objectContaining({ description: 'HTTP 404: notFound' }),
      ),
    );
    expect(toastMock.success).not.toHaveBeenCalled();
  });
});
