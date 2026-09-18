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
});
