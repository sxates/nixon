import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

// specs/0073 W3 — the app-wide half of the mover UI: the finish toast (any route) and the
// first-launch gather question, which uses the same Move recordings / Cancel dialog.

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
import { RecordingsMoveWatcher } from '@/components/RecordingsMoveWatcher';
import type { MovePlan } from '@/lib/recordings-move';

const invokeMock = vi.mocked(invoke);
const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

const PLAN: MovePlan = {
  target: '/Users/someone/Movies/nixon-recordings',
  meetings: 7,
  folders: 7,
  bytes: 2.5e9,
  sameVolumeCount: 7,
  crossVolumeCount: 0,
  crossVolumeBytes: 0,
  elsewhere: 2,
  missing: 0,
  notOwned: 0,
  protected: 0,
  unreferenced: 0,
  freeBytes: 100e9,
  enoughSpace: true,
};

function route(gather: unknown = { needsConfirmation: false, blockedReason: null, plan: { ...PLAN, meetings: 0 } }) {
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd === 'api_recordings_gather_state' ? gather : undefined),
  );
}

async function mounted() {
  render(<RecordingsMoveWatcher />);
  await waitFor(() => expect(handlers['recordings-move-finished']).toBeDefined());
}

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  for (const k of Object.keys(handlers)) delete handlers[k];
  route();
});

describe('RecordingsMoveWatcher — finish toast', () => {
  const finish = (payload: unknown) => act(() => handlers['recordings-move-finished']({ payload }));

  it('says how many recordings moved', async () => {
    await mounted();
    finish({ moved: 40, skipped: 0, failed: [], cancelled: false, removedRoots: [] });
    expect(toast.success).toHaveBeenCalledWith('Moved 40 recordings', expect.anything());
  });

  it("names the failures and offers Show, which opens the first failure's folder", async () => {
    await mounted();
    finish({
      moved: 38,
      skipped: 0,
      cancelled: false,
      removedRoots: [],
      failed: [
        { meetingId: 'm-1', title: 'Weekly sync', reason: 'The copy did not match.', folderPath: '/x/a' },
        { meetingId: 'm-2', title: 'Retro', reason: 'Disk error.', folderPath: '/x/b' },
      ],
    });
    expect(toast.warning).toHaveBeenCalledTimes(1);
    const [title, opts] = vi.mocked(toast.warning).mock.calls[0] as [
      string,
      { description: string; action: { label: string; onClick: () => void } },
    ];
    expect(title).toBe("Moved 38 of 40 — 2 couldn't be moved");
    expect(opts.description).toBe('Weekly sync: The copy did not match.');
    expect(opts.action.label).toBe('Show');
    opts.action.onClick();
    expect(invokeMock).toHaveBeenCalledWith('open_meeting_folder', { meetingId: 'm-1' });
  });

  it('stays quiet when nothing moved', async () => {
    await mounted();
    finish({ moved: 0, skipped: 0, failed: [], cancelled: false, removedRoots: [] });
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.warning).not.toHaveBeenCalled();
    expect(toast.info).not.toHaveBeenCalled();
  });
});

describe('RecordingsMoveWatcher — first-launch gather question', () => {
  it('asks before moving, from the pulled gather state (the startup event came too early)', async () => {
    route({ needsConfirmation: true, blockedReason: null, plan: PLAN });
    await mounted();

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Move your recordings?');
    expect(dialog).toHaveTextContent('7 meetings, 2.5 GB, will move to ~/Movies/nixon-recordings.');
    expect(dialog).toHaveTextContent('Includes 2 from other folders.');
    expect(calls('api_gather_recordings')).toHaveLength(0);

    fireEvent.click(within(dialog).getByRole('button', { name: 'Move recordings' }));
    await waitFor(() => expect(calls('api_gather_recordings')).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('Cancel moves nothing, and it is not asked again this session', async () => {
    route({ needsConfirmation: true, blockedReason: null, plan: PLAN });
    const { unmount } = render(<RecordingsMoveWatcher />);
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls('api_gather_recordings')).toHaveLength(0);

    // A webview remount pulls the same state again — no second question.
    unmount();
    await mounted();
    await waitFor(() => expect(calls('api_recordings_gather_state')).toHaveLength(2));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('asks when the recordings-gather-needed event arrives', async () => {
    await mounted();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    act(() => handlers['recordings-gather-needed']({ payload: { plan: PLAN } }));
    expect(await screen.findByRole('dialog')).toHaveTextContent('Move your recordings?');
  });

  it('does not ask when the gather is already agreed to', async () => {
    route({ needsConfirmation: false, blockedReason: null, plan: PLAN });
    await mounted();
    await waitFor(() => expect(calls('api_recordings_gather_state')).toHaveLength(1));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
