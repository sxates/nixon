import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

// specs/0078 review — "Who was on the mic?" used to be read by TWO `useAudioSetup`
// instances (the speakers controller and the transcript's "…" menu): two fetches, two
// `diarization-complete` listeners, and after an override only the menu's copy moved.
// This wires the real `useSpeakers` to the real `TranscriptButtonGroup` the way
// TranscriptPanel does and locks: one fetch, one listener, and the owner action's state
// and the submenu agree after an override and after a pass completes.

const { invoke, listeners } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: [] as { event: string; cb: (e: { payload: unknown }) => void }[],
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn((event: string, cb: (e: { payload: unknown }) => void) => {
    const entry = { event, cb };
    listeners.push(entry);
    return () => {
      const i = listeners.indexOf(entry);
      if (i >= 0) listeners.splice(i, 1);
    };
  }),
}));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({
    view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    enqueueMeeting: vi.fn(),
  }),
}));
// The diarization controller has its own `diarization-complete` listener (it drives the
// button's progress); stub it so the count below is the audio-setup state's alone. Its
// `identifySpeakers(start)` simply runs `start`, as the real one does first.
vi.mock('@/hooks/useDiarization', () => ({
  useDiarization: () => ({
    isRunning: false,
    stage: null,
    progressPct: null,
    downloadProgress: null,
    identifySpeakers: async (start?: () => Promise<unknown>) => {
      await start?.();
    },
  }),
}));
vi.mock('../RetranscribeDialog', () => ({ RetranscribeDialog: () => null }));

import { useSpeakers } from '@/hooks/useSpeakers';
import { TranscriptButtonGroup } from '../TranscriptButtonGroup';
import { ownerActionContext, ownerActionFor } from '../SpeakerOwnerAction';

const MEETING = 'meeting-abc';

/** Mirrors TranscriptPanel: one speakers controller feeds both the owner action and the menu. */
function MeetingView() {
  const speakers = useSpeakers({ meetingId: MEETING });
  const owner = ownerActionContext(speakers);
  return (
    <>
      <output data-testid="override">{speakers.audioSetup?.override ?? 'none'}</output>
      <output data-testid="owner-action">
        {String(ownerActionFor('spk_0', false, owner))}
      </output>
      <TranscriptButtonGroup
        transcriptCount={12}
        onCopyTranscript={vi.fn()}
        onOpenMeetingFolder={vi.fn().mockResolvedValue(undefined)}
        meetingId={MEETING}
        meetingFolderPath="/tmp/meetings/meeting-abc"
        audioSetup={speakers.audioSetup}
        onSetAudioSetup={speakers.setAudioSetup}
      />
    </>
  );
}

let stored: { override: string; resolved: string | null };

beforeEach(() => {
  invoke.mockReset();
  listeners.length = 0;
  stored = { override: 'auto', resolved: 'call' };
  invoke.mockImplementation(async (cmd: string, args?: { setup?: string }) => {
    switch (cmd) {
      case 'api_meeting_audio_status':
        return { mix: true, channels: true, compressed: false, state: 'processed' };
      case 'api_get_meeting_audio_setup':
        return { ...stored };
      case 'api_set_meeting_audio_setup':
        stored = { ...stored, override: args?.setup ?? 'auto' };
        return { started: true, alreadyRunning: false };
      case 'api_get_meeting_speakers':
        return [
          { speakerKey: 'local', displayName: 'You', isLocal: true },
          { speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false },
        ];
      case 'api_get_meeting_attendees':
        return { attendees: [], suggestion: null };
      default:
        return null;
    }
  });
});

const setupFetches = () =>
  invoke.mock.calls.filter(([c]) => c === 'api_get_meeting_audio_setup').length;
const completeListeners = () => listeners.filter((l) => l.event === 'diarization-complete');

async function chooseRoomInMenu() {
  const trigger = screen.getByRole('button', { name: 'More transcript actions' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const sub = await screen.findByRole('menuitem', { name: /who was on the mic/i });
  sub.focus();
  fireEvent.keyDown(sub, { key: 'ArrowRight' });
  const room = await screen.findByRole('menuitemradio', { name: /everyone in the room/i });
  await act(async () => {
    fireEvent.click(room);
  });
}

async function reopenSubmenuValue(): Promise<string | null> {
  const trigger = screen.getByRole('button', { name: 'More transcript actions' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const sub = await screen.findByRole('menuitem', { name: /who was on the mic/i });
  sub.focus();
  fireEvent.keyDown(sub, { key: 'ArrowRight' });
  await screen.findByRole('menuitemradio', { name: /everyone in the room/i });
  const checked = screen
    .getAllByRole('menuitemradio')
    .find((el) => el.getAttribute('aria-checked') === 'true');
  return checked?.textContent ?? null;
}

describe('one "Who was on the mic?" state per meeting view (specs/0078)', () => {
  it('fetches the setup once and listens for diarization-complete once', async () => {
    render(<MeetingView />);
    await waitFor(() => expect(screen.getByTestId('override')).toHaveTextContent('auto'));
    // Let every mount effect settle before counting.
    await act(async () => {});
    expect(setupFetches()).toBe(1);
    expect(completeListeners()).toHaveLength(1);
  });

  it('after an override and the pass it starts, the owner action and the submenu agree', async () => {
    render(<MeetingView />);
    await waitFor(() => expect(screen.getByTestId('override')).toHaveTextContent('auto'));
    // A call pass with a "You": no "This is me" on another speaker.
    await waitFor(() => expect(screen.getByTestId('owner-action')).toHaveTextContent('null'));

    await chooseRoomInMenu();
    expect(invoke).toHaveBeenCalledWith('api_set_meeting_audio_setup', {
      meetingId: MEETING,
      setup: 'room',
    });
    // The controller (which feeds the owner action) sees the override the menu set.
    await waitFor(() => expect(screen.getByTestId('override')).toHaveTextContent('room'));

    // The pass finishes as a room pass; the payload carries it, no re-fetch needed.
    const before = setupFetches();
    act(() => {
      for (const l of completeListeners()) {
        l.cb({
          payload: { meeting_id: MEETING, audioSetup: 'room', audioSetupSource: 'override' },
        });
      }
    });
    await waitFor(() => expect(screen.getByTestId('owner-action')).toHaveTextContent('mark'));
    expect(setupFetches()).toBe(before);
    expect(await reopenSubmenuValue()).toMatch(/everyone in the room/i);
  });
});
