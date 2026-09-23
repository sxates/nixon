import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { BacklogItem, BacklogItemStatus } from '@/lib/deferred-backlog';

// specs/0029 WS7.1 / specs/0072 W3 — retention deletes only media files, so a meeting can
// carry a transcript but no audio. The diarize ("Identify speakers") and re-transcribe
// ("Enhance") affordances would fail-fast with a raw error; instead the button group
// probes `api_meeting_audio_status` and gates each action on the audio it needs. These
// tests lock that gating (and that an unknown/failed probe does NOT gate).

const { invoke, enqueueMeetingSpy, listeners } = vi.hoisted(() => ({
  invoke: vi.fn(),
  enqueueMeetingSpy: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}));

type Audio = { mix: boolean; channels: boolean; compressed: boolean; state: string };
const PRESENT: Audio = { mix: true, channels: true, compressed: false, state: 'processed' };
const PURGED: Audio = { mix: false, channels: false, compressed: false, state: 'purged' };
const MISSING: Audio = { mix: false, channels: false, compressed: false, state: 'processed' };
/** invoke mock: this audio status; no processing-mode marker. */
const mockAudio = (audio: Audio | Error) =>
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_meeting_audio_status') {
      if (audio instanceof Error) throw audio;
      return audio;
    }
    return null;
  });

// specs/0045 WS3 (Task 8) — the per-meeting "Process now" button no longer owns a local
// set-once spinner; it reads the shared backlog controller (`useBacklog`). This mock
// keeps its own `useState` so a real render tree still re-renders when the fake
// `enqueueMeeting` adds an item — mirroring how the real reducer-backed provider works.
let initialBacklogItems: BacklogItem[] = [];
function setBacklogItems(items: BacklogItem[]) {
  initialBacklogItems = items;
}

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(event, cb);
    return () => listeners.delete(event);
  }),
}));
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
  it('probes api_meeting_audio_status for the meeting', async () => {
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} />);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_meeting_audio_status', {
        meetingId: 'meeting-abc',
      })
    );
  });

  it('disables Identify speakers and Enhance with retention copy when the policy deleted the audio', async () => {
    mockAudio(PURGED);
    render(<TranscriptButtonGroup {...baseProps} />);

    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    const enhance = await screen.findByRole('button', { name: /enhance/i });
    await waitFor(() => expect(identify).toBeDisabled());
    expect(enhance).toBeDisabled();
    expect(identify).toHaveAttribute('title', 'Audio deleted by your retention setting.');
    expect(enhance).toHaveAttribute('title', 'Audio deleted by your retention setting.');

    // Audio-independent affordances stay usable: the transcript is still there. They live
    // in the `…` overflow since 2026-09-21 (owner feedback — they were sitting in front of
    // the running pass), so reach them the way a user would.
    await userEvent.click(screen.getByRole('button', { name: /more transcript actions/i }));
    expect(await screen.findByRole('menuitem', { name: /^copy$/i })).not.toHaveAttribute(
      'aria-disabled',
      'true',
    );
    expect(screen.getByRole('menuitem', { name: /open folder/i })).not.toHaveAttribute(
      'aria-disabled',
      'true',
    );
  });

  // Owner feedback 2026-09-21: mid-pass the bar read "Copy · Open folder · Identifying…
  // 42% · Enhance". The two file-management affordances are now behind `…`, so the row
  // shows only what you can act on.
  it('keeps Copy and Open folder out of the front row entirely', async () => {
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());

    expect(screen.queryByRole('button', { name: /^copy$/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /open folder/i })).toBeNull();
    expect(screen.getByRole('button', { name: /more transcript actions/i })).toBeEnabled();
  });

  it('the overflow Copy is disabled when there is no transcript to copy', async () => {
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());

    await userEvent.click(screen.getByRole('button', { name: /more transcript actions/i }));
    expect(await screen.findByRole('menuitem', { name: /^copy$/i })).toHaveAttribute(
      'aria-disabled',
      'true',
    );
  });

  it('the overflow actions call through to their handlers', async () => {
    mockAudio(PRESENT);
    const onCopyTranscript = vi.fn();
    const onOpenMeetingFolder = vi.fn().mockResolvedValue(undefined);
    render(
      <TranscriptButtonGroup
        {...baseProps}
        onCopyTranscript={onCopyTranscript}
        onOpenMeetingFolder={onOpenMeetingFolder}
      />,
    );
    await waitFor(() => expect(invoke).toHaveBeenCalled());

    await userEvent.click(screen.getByRole('button', { name: /more transcript actions/i }));
    await userEvent.click(await screen.findByRole('menuitem', { name: /^copy$/i }));
    expect(onCopyTranscript).toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: /more transcript actions/i }));
    await userEvent.click(await screen.findByRole('menuitem', { name: /open folder/i }));
    expect(onOpenMeetingFolder).toHaveBeenCalled();
  });

  it('keeps both affordances enabled when audio is present', async () => {
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(
      screen.getByRole('button', { name: /identify speakers/i })
    ).toBeEnabled();
    expect(screen.getByRole('button', { name: /enhance/i })).toBeEnabled();
  });

  it('does not blame the retention setting when audio went missing some other way', async () => {
    mockAudio(MISSING);
    render(<TranscriptButtonGroup {...baseProps} />);
    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    await waitFor(() => expect(identify).toBeDisabled());
    expect(identify).toHaveAttribute('title', 'No audio recording is available for this meeting.');
  });

  it('gates each action on the audio it needs: no system channel disables only Identify speakers', async () => {
    mockAudio({ mix: true, channels: false, compressed: false, state: 'processed' });
    render(<TranscriptButtonGroup {...baseProps} />);
    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    await waitFor(() => expect(identify).toBeDisabled());
    expect(screen.getByRole('button', { name: /enhance/i })).toBeEnabled();
  });

  it('a failed identification keeps Identify speakers usable and says the audio is kept', async () => {
    mockAudio({ ...PRESENT, state: 'failed' });
    render(<TranscriptButtonGroup {...baseProps} />);
    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    await waitFor(() =>
      expect(identify).toHaveAttribute(
        'title',
        "Speaker identification didn't finish. Audio is kept so you can retry.",
      ),
    );
    expect(identify).toBeEnabled();
  });

  it('follows meeting-audio-state-changed: a purge while the page is open disables the actions', async () => {
    let audio: Audio = PRESENT;
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'api_meeting_audio_status' ? audio : null,
    );
    render(<TranscriptButtonGroup {...baseProps} />);
    const identify = await screen.findByRole('button', { name: /identify speakers/i });
    await waitFor(() => expect(listeners.has('meeting-audio-state-changed')).toBe(true));
    expect(identify).toBeEnabled();

    audio = PURGED;
    // Another meeting's event is ignored.
    listeners.get('meeting-audio-state-changed')!({ payload: { meetingId: 'other', state: 'purged' } });
    listeners.get('meeting-audio-state-changed')!({
      payload: { meetingId: 'meeting-abc', state: 'purged' },
    });
    await waitFor(() => expect(identify).toBeDisabled());
    expect(identify).toHaveAttribute('title', 'Audio deleted by your retention setting.');
  });

  it('does not gate when the probe fails (unknown state falls back to old behavior)', async () => {
    mockAudio(new Error('command not found'));
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
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    const transcribeNow = await screen.findByRole('button', { name: /transcribe now/i });
    expect(transcribeNow).toBeEnabled();
    // The beta Enhance button is replaced (not duplicated) by Transcribe now.
    expect(screen.queryByRole('button', { name: /^enhance$/i })).toBeNull();
  });

  it('shows "Transcribe now" for a sparse (below-threshold) transcript with audio', async () => {
    mockAudio(PRESENT);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={2} />);
    expect(
      await screen.findByRole('button', { name: /transcribe now/i })
    ).toBeEnabled();
  });

  it('never offers "Transcribe now" when the audio is gone', async () => {
    mockAudio(PURGED);
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });

  it('never offers "Transcribe now" while the probe is unresolved (unknown state)', async () => {
    mockAudio(new Error('command not found'));
    render(<TranscriptButtonGroup {...baseProps} transcriptCount={0} />);

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /transcribe now/i })).toBeNull();
  });

  it('keeps the plain "Enhance" affordance for fully transcribed meetings', async () => {
    mockAudio(PRESENT);
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
      if (cmd === 'api_meeting_audio_status') return audioAvailable ? PRESENT : PURGED;
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
