import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Owner feedback v0.18: the transcription bar, fixed at the bottom of the live transcript for
// the whole recording — a short status on the left ("Live", "Live · labels provisional",
// "Paused", "Paused — on battery"; the full sentence in its tooltip), "Jump to latest" in the
// middle only while scrolled up from the tail, Pause / Resume transcript on the right. The
// page's meeting bar renders at the top; the gap callout sits just above the bottom bar.

const state = vi.hoisted(() => ({
  transcripts: [] as Array<{ id: string; text: string; speaker_name?: string }>,
  gaps: { dropped: 0, skipped: 0 },
  isRecording: true,
  mode: { liveTranscription: true as boolean | null, onBattery: false },
  meetingId: 'm1' as string | null,
}));
const invokeMock = vi.fn(async (..._a: unknown[]) => ({ live_transcription_enabled: true }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
// The view reports "Jump to latest" through onJumpToLatestChange; these keys stand in for
// scrolling up from the tail and back down to it.
const jumped = vi.fn();
vi.mock('@/components/VirtualizedTranscriptView', () => ({
  VirtualizedTranscriptView: ({ onJumpToLatestChange }: { onJumpToLatestChange?: (j: (() => void) | null) => void }) => (
    <div>
      <button type="button" onClick={() => onJumpToLatestChange?.(jumped)}>
        sim scroll up
      </button>
      <button type="button" onClick={() => onJumpToLatestChange?.(null)}>
        sim at tail
      </button>
    </div>
  ),
}));
vi.mock('@/components/PermissionWarning', () => ({ PermissionWarning: () => null }));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ transcripts: state.transcripts, transcriptGaps: state.gaps }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: state.isRecording, isPaused: false }),
}));
vi.mock('@/hooks/usePermissionCheck', () => ({
  usePermissionCheck: () => ({ checkPermissions: vi.fn(), isChecking: false, hasSystemAudio: true, hasMicrophone: true }),
}));
vi.mock('@/hooks/usePlatform', () => ({ useIsLinux: () => false }));
vi.mock('@/hooks/useProcessingMode', () => ({ useProcessingMode: () => state.mode }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ activeRecordingMeetingId: state.meetingId }),
}));
vi.mock('@/hooks/useRecordEmptyPhase', () => ({ useRecordEmptyPhase: () => undefined }));
vi.mock('@/lib/live-channel-split', () => ({ expandTranscriptSegments: (t: unknown[]) => t }));

import { TranscriptPanel } from '@/app/_components/TranscriptPanel';

beforeEach(() => {
  invokeMock.mockClear();
  Object.assign(state, {
    transcripts: [],
    gaps: { dropped: 0, skipped: 0 },
    isRecording: true,
    mode: { liveTranscription: true, onBattery: false },
    meetingId: 'm1',
  });
});

const strip = () => screen.queryByTestId('transcript-status-strip');

describe('TranscriptPanel transcription bar', () => {
  it('sits at the bottom with a short "Live" and Pause transcript on the right', async () => {
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    const bar = strip()!;
    expect(bar).toHaveTextContent(/^Live/);
    expect(bar.className).toContain('border-t');
    expect(screen.getByText('Live')).toHaveAttribute('title', 'Live transcript');
    // The bar is the panel's last child: fixed at the bottom of the column.
    expect(bar.parentElement?.lastElementChild).toBe(bar);
    fireEvent.click(screen.getByRole('button', { name: 'Pause transcript' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_processing_mode', { meetingId: 'm1', mode: 'defer' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('api_apply_live_transcription_now', { enable: false });
  });

  it('once live labels exist: "Live · labels provisional", the full sentence in its tooltip', () => {
    state.transcripts = [{ id: 't1', text: 'hi', speaker_name: 'Speaker 1' }];
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    const status = screen.getByText('Live · labels provisional');
    expect(status.getAttribute('title')).toMatch(/speaker labels are finalized when you stop recording/);
    expect(strip()).not.toHaveTextContent(/finalized/);
    expect(strip()).toContainElement(screen.getByRole('button', { name: 'Pause transcript' }));
  });

  it('paused: "Paused" and Resume transcript; on battery, "Paused — on battery"', () => {
    state.mode = { liveTranscription: false, onBattery: false };
    const { unmount } = render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(screen.getByText('Paused')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Resume transcript' })).toBeInTheDocument();
    unmount();
    state.mode = { liveTranscription: false, onBattery: true };
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(screen.getByText('Paused — on battery')).toBeInTheDocument();
  });

  it('"Jump to latest" sits in the bar only while scrolled up from the tail', () => {
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(screen.queryByRole('button', { name: 'Jump to latest' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'sim scroll up' }));
    const jump = screen.getByRole('button', { name: 'Jump to latest' });
    expect(strip()).toContainElement(jump);
    fireEvent.click(jump);
    expect(jumped).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'sim at tail' }));
    expect(screen.queryByRole('button', { name: 'Jump to latest' })).toBeNull();
  });

  it('not recording: no bar, but the gap callout still shows', () => {
    state.isRecording = false;
    state.gaps = { dropped: 2, skipped: 0 };
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(strip()).toBeNull();
    expect(screen.queryByRole('button', { name: /transcript$/i })).toBeNull();
    expect(screen.getByRole('status')).toHaveTextContent(/fell behind/);
  });

  it('renders the page\'s meeting bar at the top, and the gap callout just above the bottom bar', () => {
    state.gaps = { dropped: 2, skipped: 0 };
    render(
      <TranscriptPanel isProcessingStop={false} isStopping={false} topBar={<div data-testid="meeting-bar">bar</div>} />,
    );
    const panel = strip()!.parentElement!;
    expect(panel.firstElementChild).toBe(screen.getByTestId('meeting-bar'));
    expect(strip()!.previousElementSibling).toContainElement(screen.getByRole('status'));
  });
});
