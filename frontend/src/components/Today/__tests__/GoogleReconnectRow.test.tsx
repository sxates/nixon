import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

// specs/0074 W2 / 0075 W1b — Today shows a compact "needs reconnecting" row while
// Google is connected but its grant lapsed; nothing otherwise.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const { listeners } = vi.hoisted(() => ({
  listeners: {} as Record<string, (event: unknown) => void>,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, cb: (e: unknown) => void) => {
    listeners[event] = cb;
    return Promise.resolve(() => {
      delete listeners[event];
    });
  }),
}));

const { push } = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { GoogleReconnectRow } from '@/components/Today/GoogleReconnectRow';

let status: Record<string, unknown>;

beforeEach(() => {
  invoke.mockReset();
  push.mockReset();
  for (const k of Object.keys(listeners)) delete listeners[k];
  status = { configured: true, connected: true, email: 'a@b.c', authRequired: false, calendars: [] };
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_google_calendar_status') return status;
    if (cmd === 'api_google_calendar_connect') return new Promise(() => {});
    return undefined;
  });
});

const TITLE = 'Google Calendar needs reconnecting';

describe('GoogleReconnectRow', () => {
  it('shows when Google is connected and the grant lapsed; Reconnect runs the connect flow', async () => {
    status.authRequired = true;
    render(<GoogleReconnectRow />);
    expect(await screen.findByText(TITLE)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_google_calendar_connect'));

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(push).toHaveBeenCalledWith('/settings?tab=general#calendar');
  });

  it('renders nothing while the grant is fine', async () => {
    render(<GoogleReconnectRow />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_google_calendar_status'));
    expect(screen.queryByText(TITLE)).toBeNull();
  });

  it('renders nothing when Google is not connected, even if a stale latch reads true', async () => {
    status = { ...status, connected: false, authRequired: true };
    render(<GoogleReconnectRow />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_google_calendar_status'));
    expect(screen.queryByText(TITLE)).toBeNull();
  });

  it('appears when the latch is set later, and leaves once a sync lands', async () => {
    render(<GoogleReconnectRow />);
    await waitFor(() => expect(listeners['google-calendar-auth-required']).toBeDefined());
    expect(screen.queryByText(TITLE)).toBeNull();

    status.authRequired = true;
    act(() => listeners['google-calendar-auth-required']({ payload: null }));
    expect(await screen.findByText(TITLE)).toBeInTheDocument();

    status.authRequired = false;
    act(() => listeners['google-calendar-synced']({ payload: { changed: 2 } }));
    await waitFor(() => expect(screen.queryByText(TITLE)).toBeNull());
  });
});
