import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Owner feedback v0.18: Pause / Resume transcript moved from the recording header into one
// status strip at the top of the live transcript, merged with the provisional-labels line.
// The strip is there for the whole recording (so Pause is always reachable), says
// "Provisional — …" only once a live speaker label exists, and keeps the gap callout.

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
vi.mock('@/components/VirtualizedTranscriptView', () => ({ VirtualizedTranscriptView: () => <div /> }));
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

describe('TranscriptPanel status strip', () => {
  it('shows Pause transcript while recording, before any speaker label', async () => {
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(strip()).toHaveTextContent('Live transcript');
    expect(strip()).not.toHaveTextContent(/Provisional/);
    fireEvent.click(screen.getByRole('button', { name: 'Pause transcript' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_processing_mode', { meetingId: 'm1', mode: 'defer' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('api_apply_live_transcription_now', { enable: false });
  });

  it('becomes the provisional-labels line once a live label exists, button still on the right', () => {
    state.transcripts = [{ id: 't1', text: 'hi', speaker_name: 'Speaker 1' }];
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(strip()).toHaveTextContent('Provisional — speaker labels are finalized when you stop recording.');
    expect(strip()).toContainElement(screen.getByRole('button', { name: 'Pause transcript' }));
  });

  it('paused: says so and offers Resume transcript', () => {
    state.mode = { liveTranscription: false, onBattery: false };
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(strip()).toHaveTextContent('Transcript paused');
    expect(screen.getByRole('button', { name: 'Resume transcript' })).toBeInTheDocument();
  });

  it('not recording: no strip, but the gap callout still shows', () => {
    state.isRecording = false;
    state.gaps = { dropped: 2, skipped: 0 };
    render(<TranscriptPanel isProcessingStop={false} isStopping={false} />);
    expect(strip()).toBeNull();
    expect(screen.queryByRole('button', { name: /transcript$/i })).toBeNull();
    expect(screen.getByRole('status')).toHaveTextContent(/fell behind/);
  });

  it('puts the page\'s keys after Pause transcript, at the far right', () => {
    render(
      <TranscriptPanel isProcessingStop={false} isStopping={false} stripTrailing={<button type="button">Participants</button>} />,
    );
    const pause = screen.getByRole('button', { name: 'Pause transcript' });
    const participants = screen.getByRole('button', { name: 'Participants' });
    expect(strip()).toContainElement(participants);
    expect(pause.compareDocumentPosition(participants) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
