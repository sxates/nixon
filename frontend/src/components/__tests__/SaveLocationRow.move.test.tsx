import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';

// specs/0073 W3 — Change… moves the existing recordings: plan → Move recordings / Cancel →
// api_change_recordings_folder, with inline progress + Stop that re-attach on mount, and a
// "still in another folder" line.

const { handlers } = vi.hoisted(() => ({
  handlers: {} as Record<string, (e: { payload: unknown }) => void>,
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    handlers[event] = handler;
    return Promise.resolve(() => {});
  }),
}));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() },
}));

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { SaveLocationRow } from '@/components/SaveLocationRow';
import type { RecordingPreferences } from '@/components/RecordingSettings';
import type { MovePlan, MoveStatus } from '@/lib/recordings-move';

const invokeMock = vi.mocked(invoke);

const PREFS: RecordingPreferences = {
  save_folder: '/Users/someone/Movies/nixon-recordings',
  auto_save: true,
  preferred_mic_device: null,
  preferred_system_device: null,
  retention_days: null,
  live_transcription_enabled: true,
  low_power_on_battery: true,
};

const TARGET = '/Users/someone/Recordings';

function plan(over: Partial<MovePlan> = {}): MovePlan {
  return {
    target: TARGET,
    meetings: 40,
    folders: 40,
    bytes: 12.3e9,
    sameVolumeCount: 40,
    crossVolumeCount: 0,
    crossVolumeBytes: 0,
    elsewhere: 0,
    missing: 0,
    notOwned: 0,
    protected: 0,
    unreferenced: 0,
    freeBytes: 500e9,
    enoughSpace: true,
    ...over,
  };
}

const EMPTY_GATHER = { needsConfirmation: false, blockedReason: null, plan: plan({ meetings: 0, target: PREFS.save_folder }) };

function route(overrides: Record<string, (args?: unknown) => unknown> = {}) {
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd in overrides) {
      try {
        return Promise.resolve(overrides[cmd](args));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    switch (cmd) {
      case 'api_recordings_move_status':
        return Promise.resolve(null);
      case 'api_recordings_gather_state':
        return Promise.resolve(EMPTY_GATHER);
      case 'select_recording_folder':
        return Promise.resolve(TARGET);
      case 'api_plan_recordings_move':
        return Promise.resolve(plan());
      default:
        return Promise.resolve(undefined);
    }
  });
}

function Harness({ disabled = false }: { disabled?: boolean }) {
  const [prefs, setPrefs] = useState(PREFS);
  return <SaveLocationRow preferences={prefs} setPreferences={setPrefs} disabled={disabled} />;
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

beforeEach(() => {
  vi.clearAllMocks();
  for (const k of Object.keys(handlers)) delete handlers[k];
  route();
});

describe('SaveLocationRow — Change… moves the recordings (specs/0073)', () => {
  it('is disabled while recording', async () => {
    render(<Harness disabled />);
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled();
    await waitFor(() => expect(calls('api_recordings_gather_state')).toHaveLength(1));
  });

  it('changes the folder without a dialog when nothing needs to move', async () => {
    route({ api_plan_recordings_move: () => plan({ meetings: 0, folders: 0, bytes: 0 }) });
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));

    await waitFor(() => expect(calls('api_change_recordings_folder')).toHaveLength(1));
    expect(calls('api_change_recordings_folder')[0][1]).toEqual({ target: TARGET });
    expect(screen.queryByText('Move your recordings?')).not.toBeInTheDocument();
    // The row now shows the new folder, abbreviated.
    await screen.findByText('~/Recordings');
  });

  it('asks first, with only Move recordings and Cancel — no "only new" option', async () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));

    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Move your recordings?')).toBeInTheDocument();
    expect(dialog).toHaveTextContent('40 meetings, 12.3 GB, will move to ~/Recordings.');
    expect(dialog).not.toHaveTextContent(/only new/i);
    const buttons = within(dialog)
      .getAllByRole('button')
      .map((b) => b.textContent?.trim())
      .filter((t) => t !== 'Close');
    expect(buttons).toEqual(['Cancel', 'Move recordings']);
    // Nothing has changed yet.
    expect(calls('api_change_recordings_folder')).toHaveLength(0);
  });

  it('Cancel changes nothing', async () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls('api_change_recordings_folder')).toHaveLength(0);
    expect(screen.getByText('~/Movies/nixon-recordings')).toBeInTheDocument();
  });

  it('Move recordings starts the move into the picked folder', async () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Move recordings' }));

    await waitFor(() => expect(calls('api_change_recordings_folder')).toHaveLength(1));
    expect(calls('api_change_recordings_folder')[0][1]).toEqual({ target: TARGET });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it("shows the backend's refusal text as-is and keeps the dialog open", async () => {
    const refusal =
      "Recordings need to stay on this Mac's own drive, so Nixon can't use a removable or network drive for them.";
    route({
      api_change_recordings_folder: () => {
        throw refusal;
      },
    });
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Move recordings' }));

    expect(await within(dialog).findByRole('alert')).toHaveTextContent(refusal);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText('~/Movies/nixon-recordings')).toBeInTheDocument();
  });

  it('says a cross-drive copy may take a while, and refuses without enough space', async () => {
    route({
      api_plan_recordings_move: () =>
        plan({ crossVolumeCount: 3, crossVolumeBytes: 20e9, freeBytes: 2e9, enoughSpace: false }),
    });
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('This is a copy across drives and may take a while.');
    expect(within(dialog).getByRole('alert')).toHaveTextContent(/isn't enough free space/);
    expect(within(dialog).getByRole('button', { name: 'Move recordings' })).toBeDisabled();
  });
});

describe('SaveLocationRow — progress (specs/0073)', () => {
  const running: MoveStatus = {
    done: 12,
    total: 40,
    bytesDone: 0,
    bytesTotal: 0,
    currentTitle: 'Weekly sync',
    waitingFor: null,
    target: TARGET,
  };

  it('re-attaches to a running move on mount, from api_recordings_move_status', async () => {
    route({ api_recordings_move_status: () => running });
    render(<Harness />);

    expect(await screen.findByText('Moving 13 of 40 — Weekly sync')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(calls('api_cancel_recordings_move')).toHaveLength(1));
    expect(await screen.findByRole('button', { name: 'Stopping…' })).toBeDisabled();
  });

  it('ignores a status payload it does not understand', async () => {
    route({ api_recordings_move_status: () => ({}) });
    render(<Harness />);
    await waitFor(() => expect(calls('api_recordings_move_status')).toHaveLength(1));
    expect(screen.queryByTestId('recordings-move-progress')).not.toBeInTheDocument();
  });

  it('follows progress events, says what it waits for, and clears on finish', async () => {
    render(<Harness />);
    await waitFor(() => expect(handlers['recordings-move-progress']).toBeDefined());

    act(() => handlers['recordings-move-progress']({ payload: { ...running, waitingFor: 'retranscription' } }));
    expect(await screen.findByText('Waiting for transcription to finish…')).toBeInTheDocument();

    act(() =>
      handlers['recordings-move-finished']({
        payload: { moved: 40, skipped: 0, failed: [], cancelled: false, removedRoots: [] },
      }),
    );
    await waitFor(() => expect(screen.queryByTestId('recordings-move-progress')).not.toBeInTheDocument());
  });

  it('offers "Move them here" for recordings left in another folder, with the reason', async () => {
    route({
      api_recordings_gather_state: () => ({
        needsConfirmation: false,
        blockedReason: 'There isn’t enough free space.',
        plan: plan({ meetings: 3, target: PREFS.save_folder }),
      }),
    });
    render(<Harness />);

    const line = await screen.findByTestId('recordings-left-behind');
    expect(line).toHaveTextContent('3 recordings are still in another folder — Move them here');
    expect(line).toHaveTextContent('There isn’t enough free space.');
    fireEvent.click(within(line).getByRole('button', { name: 'Move them here' }));
    await waitFor(() => expect(calls('api_gather_recordings')).toHaveLength(1));
  });

  it('shows a gather refusal as the backend text', async () => {
    route({
      api_recordings_gather_state: () => ({
        needsConfirmation: true,
        blockedReason: null,
        plan: plan({ meetings: 1, target: PREFS.save_folder }),
      }),
      api_gather_recordings: () => {
        throw 'Stop the recording to change where recordings are saved.';
      },
    });
    render(<Harness />);
    const line = await screen.findByTestId('recordings-left-behind');
    expect(line).toHaveTextContent('1 recording is still in another folder');
    fireEvent.click(within(line).getByRole('button', { name: 'Move them here' }));
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Stop the recording to change where recordings are saved.'),
    );
  });
});
