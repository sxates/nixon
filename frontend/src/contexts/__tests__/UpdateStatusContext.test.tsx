import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

type Handler = (e: { payload: unknown }) => void;
const { listeners, invoke } = vi.hoisted(() => ({
  listeners: new Map<string, Handler>(),
  invoke: vi.fn(async (_cmd: string): Promise<unknown> => ({ state: 'idle', last_checked: null })),
}));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (name: string, cb: Handler) => { listeners.set(name, cb); return () => listeners.delete(name); },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { UpdateStatusProvider, useUpdateStatus, describeStatus } from '@/contexts/UpdateStatusContext';

const wrapper = ({ children }: { children: React.ReactNode }) => <UpdateStatusProvider>{children}</UpdateStatusProvider>;
const emit = (payload: unknown) => act(() => listeners.get('update-status')?.({ payload }));

beforeEach(() => { listeners.clear(); invoke.mockClear(); });

describe('UpdateStatusProvider (specs/0058)', () => {
  it('seeds from api_get_update_status and follows the event', async () => {
    const { result } = renderHook(() => useUpdateStatus(), { wrapper });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_get_update_status'));
    emit({ state: 'downloading', version: '0.3.0', received: 5, total: 10 });
    expect(result.current.status).toEqual({ state: 'downloading', version: '0.3.0', received: 5, total: 10 });
  });

  it('checkNow invokes the command and adopts the returned status', async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'api_check_for_updates' ? { state: 'ready', version: '0.3.0', notes: 'n' } : { state: 'idle', last_checked: null });
    const { result } = renderHook(() => useUpdateStatus(), { wrapper });
    await act(() => result.current.checkNow());
    expect(result.current.status.state).toBe('ready');
  });

  it('install surfaces a refusal as error', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_install_update') throw 'Finish the recording first';
      return { state: 'ready', version: '0.3.0', notes: '' };
    });
    const { result } = renderHook(() => useUpdateStatus(), { wrapper });
    await act(() => result.current.install());
    expect(result.current.error).toBe('Finish the recording first');
  });

  it('ignores malformed payloads', async () => {
    const { result } = renderHook(() => useUpdateStatus(), { wrapper });
    emit({ nope: true });
    expect(result.current.status.state).toBe('idle');
  });
});

describe('describeStatus', () => {
  const now = new Date('2026-09-16T12:10:00Z');
  it('covers every state', () => {
    expect(describeStatus({ state: 'idle', last_checked: null }, now)).toBe('Not checked yet');
    expect(describeStatus({ state: 'idle', last_checked: '2026-09-16T12:05:00Z' }, now)).toBe('Up to date · checked 5 min ago');
    expect(describeStatus({ state: 'checking' }, now)).toBe('Checking…');
    expect(describeStatus({ state: 'downloading', version: '0.3.0', received: 1, total: null }, now)).toBe('Downloading 0.3.0…');
    expect(describeStatus({ state: 'ready', version: '0.3.0', notes: '' }, now)).toBe('0.3.0 ready — restart to update');
    expect(describeStatus({ state: 'error', message: 'x', last_checked: '2026-09-16T12:09:30Z' }, now)).toBe("Couldn't check for updates");
  });
});
