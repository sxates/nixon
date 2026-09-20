import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { UpdateStatus } from '@/contexts/UpdateStatusContext';

let status: UpdateStatus = { state: 'idle', last_checked: null };
let isRecording = false;
let error: string | null = null;
const install = vi.fn(async () => {});
vi.mock('@/contexts/UpdateStatusContext', async (orig) => ({
  ...(await orig<typeof import('@/contexts/UpdateStatusContext')>()),
  useOptionalUpdateStatus: () => ({ status, busy: false, error, checkNow: vi.fn(), install }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording }) }));

import { UpdateRow } from '@/components/Sidebar/UpdateRow';

describe('UpdateRow (specs/0058)', () => {
  it('renders nothing while idle, checking or errored', () => {
    for (const s of [
      { state: 'idle', last_checked: null },
      { state: 'checking' },
      { state: 'error', message: 'x', last_checked: '2026-09-16T00:00:00Z' },
    ] as UpdateStatus[]) {
      status = s;
      const { container, unmount } = render(<UpdateRow />);
      expect(container).toBeEmptyDOMElement();
      unmount();
    }
  });

  it('shows download progress with a percentage when the total is known', () => {
    status = { state: 'downloading', version: '0.3.0', received: 42, total: 100 };
    render(<UpdateRow />);
    expect(screen.getByText('Downloading 0.3.0 · 42%')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Restart' })).toBeNull();
  });

  it('shows download progress without a percentage when the total is unknown', () => {
    status = { state: 'downloading', version: '0.3.0', received: 42, total: null };
    render(<UpdateRow />);
    expect(screen.getByText('Downloading 0.3.0')).toBeInTheDocument();
  });

  it('offers Restart when ready and idle', () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = false;
    render(<UpdateRow />);
    expect(screen.getByText('Nixon 0.3.0 ready')).toBeInTheDocument();
    const btn = screen.getByRole('button', { name: 'Restart' });
    fireEvent.click(btn);
    expect(install).toHaveBeenCalled();
  });

  it('disables Restart while recording', () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = true;
    render(<UpdateRow />);
    const btn = screen.getByRole('button', { name: 'Restart' });
    expect(btn).toBeDisabled();
    expect(btn).toHaveAttribute('title', 'Finish the recording first');
  });

  it('shows a refused install under the row, with the full text in the title', () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = false;
    error = 'Staged update failed verification; it will be downloaded again';
    render(<UpdateRow />);
    const line = screen.getByText(error);
    expect(line).toHaveAttribute('title', error);
    expect(line).toHaveClass('truncate');
    error = null;
  });

  it('collapsed variant is a lamp-only button with an accessible name', () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = false;
    render(<UpdateRow collapsed />);
    expect(screen.getByRole('button', { name: 'Update status — Nixon 0.3.0 ready' })).toBeInTheDocument();
  });
});

// specs/0066, owner report: on the collapsed rail this lamp sits directly above the Queue's
// own amber lamp with no label, and its entire click handler was `install()`. Clicking a dot
// to find out what it is restarted the app. Identifying and acting are separate gestures now.
describe('UpdateRow collapsed — the lamp explains, it does not act', () => {
  beforeEach(() => {
    install.mockClear();
    error = null;
  });

  it('never installs on the click that opens it', async () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = false;
    render(<UpdateRow collapsed />);

    fireEvent.click(screen.getByRole('button', { name: /update status/i }));

    expect(install).not.toHaveBeenCalled();
    expect(await screen.findByText('Nixon 0.3.0 ready')).toBeInTheDocument();
  });

  it('installs only from the labelled action inside the flyout', async () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = false;
    render(<UpdateRow collapsed />);

    fireEvent.click(screen.getByRole('button', { name: /update status/i }));
    fireEvent.click(await screen.findByRole('button', { name: 'Restart to update' }));

    expect(install).toHaveBeenCalled();
  });

  it('says why it will not restart mid-recording, rather than hiding it in a tooltip', async () => {
    status = { state: 'ready', version: '0.3.0', notes: '', last_checked: null };
    isRecording = true;
    render(<UpdateRow collapsed />);

    fireEvent.click(screen.getByRole('button', { name: /update status/i }));

    const restart = await screen.findByRole('button', { name: 'Restart to update' });
    expect(restart).toBeDisabled();
    expect(screen.getByText(/Finish the recording first/)).toBeInTheDocument();
    fireEvent.click(restart);
    expect(install).not.toHaveBeenCalled();
  });

  it('reports progress without offering a restart that is not ready', async () => {
    status = { state: 'downloading', version: '0.3.0', received: 42, total: 100 };
    isRecording = false;
    render(<UpdateRow collapsed />);

    fireEvent.click(screen.getByRole('button', { name: /update status/i }));

    expect(await screen.findByText('Downloading 0.3.0 · 42%')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Restart to update' })).toBeNull();
  });
});
