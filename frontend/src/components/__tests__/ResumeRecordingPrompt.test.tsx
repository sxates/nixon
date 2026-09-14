import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

// specs/0037 — relaunch recovery prompt. On mount it scans for interrupted recordings
// (api_list_interrupted_recordings); when the list is non-empty it renders a modal, and
// Discard fires api_discard_interrupted_recording for that meeting. Mocks invoke +
// next/navigation + RecordingStateContext the same way the other component tests do.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const pushMock = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

// Not recording by default — the prompt is suppressed while a recording is live.
const recordingStateMock = { isRecording: false };
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => recordingStateMock,
}));

import { invoke } from '@tauri-apps/api/core';
import { armResumeRecording, RESUME_RECORDING_KEY } from '@/lib/resume-recording';
import ResumeRecordingPrompt from '@/components/ResumeRecordingPrompt';

const invokeMock = vi.mocked(invoke);

function routeInvoke(list: unknown[]) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_list_interrupted_recordings':
        return Promise.resolve(list);
      case 'api_discard_interrupted_recording':
        return Promise.resolve(undefined);
      default:
        return Promise.resolve(undefined);
    }
  });
}

const INTERRUPTED = {
  meetingId: 'meeting-crash',
  folderPath: '/recordings/crash',
  meetingName: 'Standup',
  startedAt: '2026-07-05T10:00:00Z',
  segmentCount: 1,
};

describe('ResumeRecordingPrompt (specs/0037)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    recordingStateMock.isRecording = false;
  });

  it('renders the prompt when the interrupted list is non-empty', async () => {
    routeInvoke([INTERRUPTED]);
    render(<ResumeRecordingPrompt />);

    expect(await screen.findByText('Unfinished recording')).toBeInTheDocument();
    // Body shows the meeting name.
    expect(screen.getByText(/Standup/)).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('api_list_interrupted_recordings');
  });

  it('renders nothing when there are no interrupted recordings', async () => {
    routeInvoke([]);
    render(<ResumeRecordingPrompt />);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_list_interrupted_recordings'));
    expect(screen.queryByText('Unfinished recording')).not.toBeInTheDocument();
  });

  it('Discard calls api_discard_interrupted_recording for that meeting', async () => {
    routeInvoke([INTERRUPTED]);
    render(<ResumeRecordingPrompt />);

    await screen.findByText('Unfinished recording');
    fireEvent.click(screen.getByRole('button', { name: /discard/i }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_discard_interrupted_recording', {
        meetingId: 'meeting-crash',
      }),
    );
  });

  it('Resume arms the resumeRecording stash and navigates to /record', async () => {
    routeInvoke([INTERRUPTED]);
    render(<ResumeRecordingPrompt />);

    await screen.findByText('Unfinished recording');
    fireEvent.click(screen.getByRole('button', { name: /resume recording/i }));

    expect(pushMock).toHaveBeenCalledWith('/record');
    const stash = JSON.parse(sessionStorage.getItem(RESUME_RECORDING_KEY) ?? 'null');
    expect(stash).toMatchObject({ meetingId: 'meeting-crash', folderPath: '/recordings/crash' });
  });
});

// Lightweight guard that the shared stash contract the /record consumer relies on is
// stable (specs/0037): armResumeRecording writes the exact key the hook consumes.
describe('resume-recording stash', () => {
  beforeEach(() => sessionStorage.clear());
  it('armResumeRecording writes the resumeRecording key', () => {
    armResumeRecording({ meetingId: 'm1', folderPath: '/f', meetingName: 'Name' });
    expect(JSON.parse(sessionStorage.getItem(RESUME_RECORDING_KEY) ?? 'null')).toEqual({
      meetingId: 'm1',
      folderPath: '/f',
      meetingName: 'Name',
    });
  });
});
