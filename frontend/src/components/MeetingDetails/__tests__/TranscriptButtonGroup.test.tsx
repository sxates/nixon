import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { BacklogItem, BacklogItemStatus } from '@/lib/deferred-backlog';

// specs/0029 WS7.1 — the retention sweep deletes only media files, so a meeting can
// carry a transcript but no audio. The diarize ("Identify speakers") and re-transcribe
// ("Enhance") affordances would fail-fast with a raw error; instead the button group
// probes `api_meeting_audio_available` and renders a friendly disabled state. These
// tests lock that gating (and that an unknown/failed probe does NOT gate).

const { invoke, enqueueMeetingSpy } = vi.hoisted(() => ({
  invoke: vi.fn(),
  enqueueMeetingSpy: vi.fn(),
}));

// specs/0045 WS3 (Task 8) — the per-meeting "Process now" button no longer owns a local
// set-once spinner; it reads the shared backlog controller (`useBacklog`). This mock
// keeps its own `useState` so a real render tree still re-renders when the fake
// `enqueueMeeting` adds an item — mirroring how the real reducer-backed provider works.
let initialBacklogItems: BacklogItem[] = [];
function setBacklogItems(items: BacklogItem[]) {
  initialBacklogItems = items;
}

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => {
    const [items, setItems] = useState<BacklogItem[]>(() => initialBacklogItems);
    const enqueueMeeting = (meetingId: string, opts?: { force?: boolean }) => {
      enqueueMeetingSpy(meetingId, opts);
      setItems((prev) => {
        if (prev.some((i) => i.meeting.id === meetingId)) return prev;
        return [
          ...prev,
          {
            meeting: { id: meetingId, title: '', folderPath: '', transcriptCount: 0 },
            status: 'waiting' as BacklogItemStatus,
          },
        ];
      });
    };
    return {
      view: { items, pendingCount: items.length, processing: false, active: null, activeOrdinal: 0, total: items.length },
      enqueueMeeting,
      stop: vi.fn(),
      dismissDone: vi.fn(),
      startNow: vi.fn(),
    };
  },
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ betaFeatures: { importAndRetranscribe: true } }),
}));
vi.mock('@/hooks/useDiarization', () => ({
  useDiarization: () => ({
    isRunning: false,
    stage: null,
    progressPct: null,
    identifySpeakers: vi.fn(),
  }),
}));
vi.mock('../RetranscribeDialog', () => ({
  RetranscribeDialog: () => null,
}));

import { TranscriptButtonGroup } from '../TranscriptButtonGroup';

const baseProps = {
  transcriptCount: 12,
  onCopyTranscript: vi.fn(),
  onOpenMeetingFolder: vi.fn().mockResolvedValue(undefined),
  meetingId: 'meeting-abc',
  meetingFolderPath: '/tmp/meetings/meeting-abc',
};

beforeEach(() => {
  invoke.mockReset();
  enqueueMeetingSpy.mockReset();
  setBacklogItems([]);
});

describe('TranscriptButtonGroup audio-availability gating (WS7.1)', () => {
  it('probes api_meeting_audio_available for the meeting', async () => {
    invoke.mockResolvedValue(true);
    render(<TranscriptButtonGroup {...baseProps} />);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_meeting_audio_available', {
        meetingId: 'meeting-abc',
      })
    );
  });

  it('disables Identify speakers and Enhance with retention copy when audio is gone', async () => {
    invoke.mockResolvedValue(false);
    render(<TranscriptButtonGroup {...baseProps} />);

    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    const enhance = await screen.findByRole('button', { name: /enhance/i });
    await waitFor(() => expect(identify).toBeDisabled());
    expect(enhance).toBeDisabled();
    expect(identify).toHaveAttribute(
      'title',
      expect.stringMatching(/no audio recording available.*retention/i)
    );
    expect(enhance).toHaveAttribute(
      'title',
      expect.stringMatching(/no audio recording available.*retention/i)
    );

    // Audio-independent affordances stay usable: the transcript is still there.
    expect(screen.getByRole('button', { name: /copy/i })).toBeEnabled();
    expect(screen.getByRole('button', { name: /open folder/i })).toBeEnabled();
  });

  it('keeps both affordances enabled when audio is present', async () => {
    invoke.mockResolvedValue(true);
    render(<TranscriptButtonGroup {...baseProps} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(
      screen.getByRole('button', { name: /identify speakers/i })
    ).toBeEnabled();
    expect(screen.getByRole('button', { name: /enhance/i })).toBeEnabled();
  });

  it('does not gate when the probe fails (unknown state falls back to old behavior)', async () => {
    invoke.mockRejectedValue(new Error('command not found'));
    render(<TranscriptButtonGroup {...baseProps} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(
      screen.getByRole('button', { name: /identify speakers/i })
    ).toBeEnabled();
    expect(screen.getByRole('button', { name: /enhance/i })).toBeEnabled();
  });
});

// specs/0029 WS7.2 — record-only meetings (live transcription off) have audio but
// no/sparse transcript rows: surface "Transcribe now" (first-time deferred
// transcription) in place of the beta "Enhance" retranscribe affordance.
describe('TranscriptButtonGroup deferred transcription (WS7.2)', () => {
  it('shows "Transcribe now" for a transcript-empty meeting with audio', async () => {
    invoke.mockResolvedValue(true);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    const transcribeNow = await screen.findByRole('button', { name: /transcribe now/i });
    expect(transcribeNow).toBeEnabled();
    // The beta Enhance button is replaced (not duplicated) by Transcribe now.
    expect(screen.queryByRole('button', { name: /^enhance$/i })).toBeNull();
  });

  it('shows "Transcribe now" for a sparse (below-threshold) transcript with audio', async () => {
    invoke.mockResolvedValue(true);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={2} />);
    expect(
      await screen.findByRole('button', { name: /transcribe now/i })
    ).toBeEnabled();
  });

  it('never offers "Transcribe now" when the audio is gone', async () => {
    invoke.mockResolvedValue(false);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });

  it('never offers "Transcribe now" while the probe is unresolved (unknown state)', async () => {
    invoke.mockRejectedValue(new Error('command not found'));
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });

  it('keeps the plain "Enhance" affordance for fully transcribed meetings', async () => {
    invoke.mockResolvedValue(true);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={12} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: /enhance/i })).toBeEnabled();
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });
});

// 1.10 feedback / specs/0045 WS3 Task 8 — a battery-deferred meeting (processing_mode=
// 'defer') runs NOTHING automatically; its manual trigger is a "Process now" button
// that ENQUEUES the meeting on the shared backlog controller (never a fire-and-forget
// window event) and reads that same controller's state back, so the button reflects
// reality — including surviving navigation — instead of a local set-once spinner.
describe('TranscriptButtonGroup deferred processing (1.10 feedback / specs/0045 Task 8)', () => {
  /** invoke mock: audio present; meeting marked deferred. */
  const mockDeferredMeeting = (mode: string | null, audioAvailable = true) => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_meeting_audio_available') return audioAvailable;
      if (cmd === 'api_get_meeting_processing_mode') return mode;
      return null;
    });
  };

  it('shows "Process now" (not "Transcribe now") for a deferred meeting with audio', async () => {
    mockDeferredMeeting('defer');
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    const processNow = await screen.findByRole('button', { name: /process now/i });
    expect(processNow).toBeEnabled();
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });

  it('enqueues the meeting on the shared backlog controller (force: true) on click, never a window event', async () => {
    mockDeferredMeeting('defer');
    const heard: string[] = [];
    const listener = (event: Event) => {
      heard.push((event as CustomEvent<{ meetingId?: string }>).detail?.meetingId ?? '');
    };
    window.addEventListener('process-deferred-meeting-now', listener);
    try {
      render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
      const processNow = await screen.findByRole('button', { name: /process now/i });
      await userEvent.click(processNow);
      expect(enqueueMeetingSpy).toHaveBeenCalledWith('meeting-abc', { force: true });
      // The old silent-no-op path is gone entirely — nothing dispatches the event.
      expect(heard).toEqual([]);
    } finally {
      window.removeEventListener('process-deferred-meeting-now', listener);
    }
  });

  it('shows "Queued" and disables the button once the shared controller reports "waiting"', async () => {
    mockDeferredMeeting('defer');
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
    const processNow = await screen.findByRole('button', { name: /process now/i });
    await userEvent.click(processNow);
    const queued = await screen.findByRole('button', { name: /queued/i });
    expect(queued).toBeDisabled();
  });

  it('shows "Processing…" and disables the button when the shared controller reports an active stage', async () => {
    mockDeferredMeeting('defer');
    setBacklogItems([
      {
        meeting: { id: 'meeting-abc', title: 't', folderPath: '/tmp', transcriptCount: 0 },
        status: 'diarizing',
      },
    ]);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
    const processing = await screen.findByRole('button', { name: /processing/i });
    expect(processing).toBeDisabled();
  });

  it('never offers "Process now" when the audio is gone', async () => {
    mockDeferredMeeting('defer', false);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /process now/i })).toBeNull();
  });

  it('keeps plain "Transcribe now" for a record-only meeting with NO defer marker', async () => {
    // e.g. live transcription globally off (not battery deferral): transcription
    // is still the user's explicit, single-step affordance.
    mockDeferredMeeting(null);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
    expect(await screen.findByRole('button', { name: /transcribe now/i })).toBeEnabled();
    expect(screen.queryByRole('button', { name: /process now/i })).toBeNull();
  });
});
