import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const install = vi.fn(async () => {});
let isRecording = false;
let listener: ((e: { payload: unknown }) => void) | null = null;
let listenedEvent: string | null = null;

vi.mock('@/contexts/UpdateStatusContext', () => ({
  useOptionalUpdateStatus: () => ({
    status: { state: 'ready', version: '0.8.0', notes: '', last_checked: null },
    busy: false,
    error: null,
    checkNow: vi.fn(),
    install,
  }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording }),
}));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: async () => '0.7.0' }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, cb: (e: { payload: unknown }) => void) => {
    listenedEvent = event;
    listener = cb;
    return () => { listener = null; };
  },
}));
const { toastError } = vi.hoisted(() => ({ toastError: vi.fn() }));
vi.mock('sonner', () => ({ toast: { error: toastError } }));

import { RestartConfirmProvider, useRestartConfirm } from '@/contexts/RestartConfirmContext';

function Asker() {
  const { request } = useRestartConfirm();
  return <button onClick={request}>ask</button>;
}
const wrap = () =>
  render(
    <RestartConfirmProvider>
      <Asker />
    </RestartConfirmProvider>,
  );

describe('RestartConfirmProvider (specs/0069 W5)', () => {
  beforeEach(() => {
    install.mockClear();
    toastError.mockClear();
    isRecording = false;
    listener = null;
    listenedEvent = null;
  });

  // Review finding, fix round 2: the mock above used to ignore the event name it was
  // called with, so renaming either `EVENT_CONFIRM_RESTART` (updater/mod.rs) or the tray's
  // matching hardcoded string would leave the tray silently disconnected from this provider
  // while every test here stayed green. Pinning the literal here means a rename on either
  // side shows up as a failure instead of a runtime no-op.
  it('listens for the exact event name the tray emits ("update-confirm-restart")', () => {
    wrap();
    expect(listenedEvent).toBe('update-confirm-restart');
  });

  it('asking opens a dialog and installs nothing', async () => {
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(install).not.toHaveBeenCalled();
  });

  it('names both versions and says the app will close', async () => {
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    const dialog = await screen.findByRole('dialog');
    await waitFor(() => expect(dialog).toHaveTextContent('0.7.0'));
    expect(dialog).toHaveTextContent('0.8.0');
    expect(dialog).toHaveTextContent(/close and reopen/i);
  });

  it('installs only after Restart now', async () => {
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    fireEvent.click(await screen.findByRole('button', { name: /restart now/i }));
    await waitFor(() => expect(install).toHaveBeenCalledTimes(1));
  });

  it('Cancel closes and installs nothing', async () => {
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    fireEvent.click(await screen.findByRole('button', { name: /cancel/i }));
    expect(install).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('the tray asks through the same dialog (specs/0069 W5)', async () => {
    wrap();
    expect(listener).not.toBeNull();
    listener?.({ payload: null });
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(install).not.toHaveBeenCalled();
  });

  it('will not open at all while recording, and says why instead of staying silent', () => {
    // Review finding, fix round 2: this test used to assert only the dialog's absence,
    // which is exactly what a silently-dropped request also looks like. A refusal now says
    // so via the house `sonner` toast, reusing the recording copy from `UpdateRow.tsx`'s
    // popover verbatim.
    isRecording = true;
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(toastError).toHaveBeenCalledWith("Finish the recording first — Nixon won't restart mid-take.");
    expect(install).not.toHaveBeenCalled();
  });

  it('the tray click also surfaces the refusal, not silence, when it lands while recording', () => {
    isRecording = true;
    wrap();
    listener?.({ payload: null });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(toastError).toHaveBeenCalledWith("Finish the recording first — Nixon won't restart mid-take.");
    expect(install).not.toHaveBeenCalled();
  });
});
