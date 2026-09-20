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
const request = vi.fn();
vi.mock('@/contexts/UpdateStatusContext', async (orig) => ({
  ...(await orig<typeof import('@/contexts/UpdateStatusContext')>()),
  useOptionalUpdateStatus: () => ({ status, busy: false, error: null, checkNow, install }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording: false }) }));
// specs/0069 W5 — About's Restart to update button now asks the app-wide confirmation
// (RestartConfirmContext) instead of installing directly.
vi.mock('@/contexts/RestartConfirmContext', () => ({ useRestartConfirm: () => ({ request, canRestart: true }) }));
// AnswerMarkdown reaches for the router to open a cited meeting; release notes carry no
// citations, but the hook still runs.
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));

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
    expect(request).toHaveBeenCalledTimes(1);
    expect(install).not.toHaveBeenCalled();
  });

  // specs/0066 W4 — the release body IS markdown (the changelog's own sections, published
  // verbatim), and it used to be split one bullet per line: `###` headings rendered as
  // bullets with the hashes still on them, and blank lines vanished.
  it('renders the release notes as markdown, not one bullet per line', () => {
    status = {
      state: 'ready',
      version: '0.6.0',
      notes: '### Changed\n\n- The sidebar Home is now Today\n- Queue moved to the sidebar\n\n### Fixed\n\n- REC is readable on hold',
      last_checked: null,
    };
    const { container } = render(<About />);

    const heading = screen.getByText('Changed');
    expect(heading.tagName).toBe('H3');
    expect(heading.textContent).not.toContain('#');
    expect(screen.getByText('Fixed').tagName).toBe('H3');

    // The bullets are real list items, and the headings are NOT among them.
    const items = Array.from(container.querySelectorAll('li')).map((li) => li.textContent);
    expect(items).toContain('The sidebar Home is now Today');
    expect(items).toContain('REC is readable on hold');
    expect(items).not.toContain('### Changed');
  });

  it('centres the page in the settings pane', () => {
    status = { state: 'idle', last_checked: null };
    const { container } = render(<About />);
    expect(container.firstElementChild?.className).toContain('mx-auto');
  });

  it('shows the error message when a check failed', () => {
    status = { state: 'error', message: 'offline', last_checked: '2026-09-16T00:00:00Z' };
    render(<About />);
    expect(screen.getByText("Couldn't check for updates")).toBeInTheDocument();
    expect(screen.getByText('offline')).toBeInTheDocument();
  });
});
