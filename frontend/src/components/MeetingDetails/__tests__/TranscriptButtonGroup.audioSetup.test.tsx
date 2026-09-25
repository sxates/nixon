import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

// specs/0078 W3 — "Who was on the mic?" in the transcript's "…" menu. Locks: it is
// hidden for imported / notes-only meetings and for meetings whose channels are gone,
// disabled while a pass runs, and a choice goes through the diarization controller
// (so progress shows) with a start function that sets the right override. The state and
// the setter come in as props from the speakers controller (one instance per meeting view;
// see TranscriptButtonGroup.sharedAudioSetup.test.tsx).

const { invoke, identifySpeakers, diarization, setAudioSetup } = vi.hoisted(() => ({
  invoke: vi.fn(),
  identifySpeakers: vi.fn(),
  setAudioSetup: vi.fn(),
  diarization: { isRunning: false },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn(() => () => {}) }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({
    view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    enqueueMeeting: vi.fn(),
  }),
}));
vi.mock('@/hooks/useDiarization', () => ({
  useDiarization: () => ({
    isRunning: diarization.isRunning,
    stage: null,
    progressPct: null,
    downloadProgress: null,
    identifySpeakers,
  }),
}));
vi.mock('../RetranscribeDialog', () => ({ RetranscribeDialog: () => null }));

import { TranscriptButtonGroup } from '../TranscriptButtonGroup';

type Audio = { mix: boolean; channels: boolean; compressed: boolean; state: string };
const PRESENT: Audio = { mix: true, channels: true, compressed: false, state: 'processed' };
const PURGED: Audio = { mix: false, channels: false, compressed: false, state: 'purged' };

function mockBackend(audio: Audio = PRESENT) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_meeting_audio_status') return audio;
    return null;
  });
}

const baseProps = {
  transcriptCount: 12,
  onCopyTranscript: vi.fn(),
  onOpenMeetingFolder: vi.fn().mockResolvedValue(undefined),
  meetingId: 'meeting-abc',
  meetingFolderPath: '/tmp/meetings/meeting-abc',
  audioSetup: { override: 'auto' as const, resolved: 'room' as const },
  onSetAudioSetup: setAudioSetup,
};

async function openMoreMenu() {
  const trigger = screen.getByRole('button', { name: 'More transcript actions' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  await screen.findByRole('menuitem', { name: 'Copy' });
}

async function openMicSubmenu() {
  const sub = await screen.findByRole('menuitem', { name: /who was on the mic/i });
  sub.focus();
  fireEvent.keyDown(sub, { key: 'ArrowRight' });
  return screen.findByRole('menuitemradio', { name: /everyone in the room/i });
}

beforeEach(() => {
  invoke.mockReset();
  identifySpeakers.mockReset();
  setAudioSetup.mockReset().mockResolvedValue({ started: true, alreadyRunning: false });
  diarization.isRunning = false;
  mockBackend();
});

describe('TranscriptButtonGroup — "Who was on the mic?" (specs/0078)', () => {
  it('offers the submenu for a recorded meeting, with the detected caption', async () => {
    render(<TranscriptButtonGroup {...baseProps} meetingOrigin="recorded" />);
    await openMoreMenu();
    await openMicSubmenu();
    expect(screen.getByRole('menuitemradio', { name: /detect automatically/i })).toHaveTextContent(
      'detected: in a room',
    );
    expect(screen.getByRole('menuitemradio', { name: /detect automatically/i })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  it.each(['imported', 'notes_only'])('hides the submenu for a %s meeting', async (origin) => {
    render(<TranscriptButtonGroup {...baseProps} meetingOrigin={origin} />);
    await openMoreMenu();
    expect(screen.queryByRole('menuitem', { name: /who was on the mic/i })).toBeNull();
  });

  it('hides the submenu when no setter is passed down', async () => {
    render(<TranscriptButtonGroup {...baseProps} onSetAudioSetup={undefined} />);
    await openMoreMenu();
    expect(screen.queryByRole('menuitem', { name: /who was on the mic/i })).toBeNull();
  });

  it('does not fetch the audio setup itself (the speakers controller owns it)', async () => {
    render(<TranscriptButtonGroup {...baseProps} />);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_meeting_audio_status', { meetingId: 'meeting-abc' }),
    );
    expect(invoke).not.toHaveBeenCalledWith('api_get_meeting_audio_setup', expect.anything());
  });

  it('hides the submenu when the meeting audio is gone', async () => {
    mockBackend(PURGED);
    render(<TranscriptButtonGroup {...baseProps} />);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_meeting_audio_status', { meetingId: 'meeting-abc' }),
    );
    // Let the probe result render before opening the menu.
    await act(async () => {});
    await openMoreMenu();
    expect(screen.queryByRole('menuitem', { name: /who was on the mic/i })).toBeNull();
  });

  it('disables the submenu while a pass is running', async () => {
    diarization.isRunning = true;
    render(<TranscriptButtonGroup {...baseProps} />);
    await openMoreMenu();
    const sub = await screen.findByRole('menuitem', { name: /who was on the mic/i });
    expect(sub).toHaveAttribute('data-disabled');
  });

  it('a choice starts the pass through the diarization controller with the right override', async () => {
    render(<TranscriptButtonGroup {...baseProps} />);
    await openMoreMenu();
    const room = await openMicSubmenu();
    fireEvent.click(room);

    expect(identifySpeakers).toHaveBeenCalledTimes(1);
    const start = identifySpeakers.mock.calls[0][0] as () => Promise<unknown>;
    expect(typeof start).toBe('function');
    await act(async () => {
      await start();
    });
    expect(setAudioSetup).toHaveBeenCalledWith('room');
  });
});
