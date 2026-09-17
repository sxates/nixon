import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import type { UpdateStatus } from '@/contexts/UpdateStatusContext';

// specs/0058 Task 5 — the About panel's update status line, "Check for updates"
// and (when a build is ready) "Restart to update" button + release notes.

vi.mock('@tauri-apps/api/app', () => ({ getVersion: async () => '0.3.0' }));
let status: UpdateStatus = { state: 'idle', last_checked: null };
const checkNow = vi.fn(async () => {});
const install = vi.fn(async () => {});
vi.mock('@/contexts/UpdateStatusContext', async (orig) => ({
  ...(await orig<typeof import('@/contexts/UpdateStatusContext')>()),
  useOptionalUpdateStatus: () => ({ status, busy: false, error: null, checkNow, install }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording: false }) }));

import { About } from '@/components/About';

describe('About updates (specs/0058)', () => {
  it('shows the status line and a Check for updates button', () => {
    status = { state: 'idle', last_checked: null };
    render(<About />);
    expect(screen.getByText('Not checked yet')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(checkNow).toHaveBeenCalled();
  });

  it('shows notes and a Restart to update button when ready', () => {
    status = { state: 'ready', version: '0.3.1', notes: '- Fixed a thing', last_checked: null };
    render(<About />);
    expect(screen.getByText('0.3.1 ready — restart to update')).toBeInTheDocument();
    expect(screen.getByText('Fixed a thing')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }));
    expect(install).toHaveBeenCalled();
  });

  it('shows the error message when a check failed', () => {
    status = { state: 'error', message: 'offline', last_checked: '2026-09-16T00:00:00Z' };
    render(<About />);
    expect(screen.getByText("Couldn't check for updates")).toBeInTheDocument();
    expect(screen.getByText('offline')).toBeInTheDocument();
  });
});
