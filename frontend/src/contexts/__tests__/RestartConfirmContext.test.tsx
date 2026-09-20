import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const install = vi.fn(async () => {});
let isRecording = false;
let listener: ((e: { payload: unknown }) => void) | null = null;

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
  safeListen: (_event: string, cb: (e: { payload: unknown }) => void) => {
    listener = cb;
    return () => { listener = null; };
  },
}));

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
    isRecording = false;
    listener = null;
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

  it('will not open at all while recording', () => {
    isRecording = true;
    wrap();
    fireEvent.click(screen.getByRole('button', { name: 'ask' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    listener?.({ payload: null });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(install).not.toHaveBeenCalled();
  });
});
