import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';

// specs/0061 W1 Task 3 — the onboarding Calendar step and `CalendarSettings`
// both need the Google Calendar connect flow (status + connect + the
// sequence guard + the two toasts). This hook is the single implementation
// both call sites share.

const { getGoogleCalendarStatus, connectGoogleCalendar } = vi.hoisted(() => ({
  getGoogleCalendarStatus: vi.fn(),
  connectGoogleCalendar: vi.fn(),
}));
vi.mock('@/lib/googleCalendar', () => ({
  getGoogleCalendarStatus,
  connectGoogleCalendar,
}));

const { toastMock } = vi.hoisted(() => ({
  toastMock: { success: vi.fn(), error: vi.fn() },
}));
vi.mock('sonner', () => ({ toast: toastMock }));

import { useGoogleCalendarConnect } from '@/hooks/useGoogleCalendarConnect';

beforeEach(() => {
  getGoogleCalendarStatus.mockReset();
  connectGoogleCalendar.mockReset();
  toastMock.success.mockReset();
  toastMock.error.mockReset();
  getGoogleCalendarStatus.mockResolvedValue({ configured: true, connected: false });
  connectGoogleCalendar.mockResolvedValue({ ok: true, email: 'user@halden.example' });
});

describe('useGoogleCalendarConnect', () => {
  it('loads status on mount', async () => {
    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() =>
      expect(result.current.status).toEqual({ configured: true, connected: false }),
    );
  });

  it('toggles connecting and toasts success on a successful connect()', async () => {
    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() => expect(result.current.status).not.toBeNull());

    expect(result.current.connecting).toBe(false);

    let outcome: Awaited<ReturnType<typeof result.current.connect>> | undefined;
    await act(async () => {
      outcome = await result.current.connect();
    });

    expect(outcome).toEqual({ ok: true, email: 'user@halden.example' });
    expect(result.current.connecting).toBe(false);
    expect(toastMock.success).toHaveBeenCalledWith('Google Calendar connected', {
      description: 'user@halden.example',
    });
  });

  it('sets connecting true while the connect call is in flight', async () => {
    let resolveConnect: (value: { ok: true; email: string }) => void = () => {};
    connectGoogleCalendar.mockReturnValue(
      new Promise((resolve) => {
        resolveConnect = resolve;
      }),
    );

    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() => expect(result.current.status).not.toBeNull());

    let connectPromise: ReturnType<typeof result.current.connect>;
    act(() => {
      connectPromise = result.current.connect();
    });

    await waitFor(() => expect(result.current.connecting).toBe(true));

    await act(async () => {
      resolveConnect({ ok: true, email: 'user@halden.example' });
      await connectPromise;
    });

    expect(result.current.connecting).toBe(false);
  });

  it('toasts an error and does not throw on a failed connect()', async () => {
    connectGoogleCalendar.mockResolvedValue({ ok: false, error: 'nope' });
    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() => expect(result.current.status).not.toBeNull());

    let outcome: Awaited<ReturnType<typeof result.current.connect>> | undefined;
    await act(async () => {
      outcome = await result.current.connect();
    });

    expect(outcome).toEqual({ ok: false, error: 'nope' });
    expect(toastMock.error).toHaveBeenCalledWith('Could not connect Google Calendar', {
      description: 'nope',
    });
    expect(toastMock.success).not.toHaveBeenCalled();
  });

  // specs/0061 W1 Task 3 fix (controller ruling R14) — a "Cancel"/"Dismiss"
  // button hides the pending UI, but the backend connect invoke keeps
  // running underneath (it can't be aborted) for up to 5 minutes. Before
  // this fix, its eventual resolution still toasted "Google Calendar
  // connected" and refreshed status — minutes after the user gave up, and
  // even if they'd navigated away (sonner renders at the app root). cancel()
  // must make that resolution fully silent.
  it('cancel() makes a later resolution of the cancelled connect() silent: no toast, no status refresh', async () => {
    let resolveConnect: (value: { ok: true; email: string }) => void = () => {};
    connectGoogleCalendar.mockReturnValue(
      new Promise((resolve) => {
        resolveConnect = resolve;
      }),
    );

    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    getGoogleCalendarStatus.mockClear(); // drop the mount-time call

    let connectPromise: ReturnType<typeof result.current.connect>;
    act(() => {
      connectPromise = result.current.connect();
    });
    await waitFor(() => expect(result.current.connecting).toBe(true));

    act(() => {
      result.current.cancel();
    });
    // The pending UI drops immediately, before the backend call resolves.
    expect(result.current.connecting).toBe(false);

    await act(async () => {
      resolveConnect({ ok: true, email: 'user@halden.example' });
      await connectPromise;
    });

    expect(toastMock.success).not.toHaveBeenCalled();
    expect(toastMock.error).not.toHaveBeenCalled();
    expect(getGoogleCalendarStatus).not.toHaveBeenCalled();
    expect(result.current.connecting).toBe(false);
  });

  it('cancel() has no effect on toast/refresh for a connect() that resolves BEFORE it is cancelled', async () => {
    // A cancel() that lands after a connect() already settled (fast success,
    // slow click) must not retroactively silence it.
    const { result } = renderHook(() => useGoogleCalendarConnect());
    await waitFor(() => expect(result.current.status).not.toBeNull());

    await act(async () => {
      await result.current.connect();
    });
    expect(toastMock.success).toHaveBeenCalledTimes(1);

    act(() => {
      result.current.cancel();
    });
    expect(toastMock.success).toHaveBeenCalledTimes(1);
  });
});
