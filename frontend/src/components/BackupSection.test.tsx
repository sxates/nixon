import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

const { invoke, toast } = vi.hoisted(() => ({
  invoke: vi.fn(),
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('sonner', () => ({ toast }));

import { BackupSection } from '@/components/BackupSection';

const baseSettings = {
  enabled: false,
  folder: null as string | null,
  includeAudio: false,
  keep: 7,
  lastSuccessAt: null as string | null,
  lastError: null as string | null,
  lastSize: null as number | null,
};

const previewOf = (localMeetings: number) => ({
  createdAt: '2026-11-24T10:00:00Z',
  appVersion: '0.18.2',
  includesAudio: false,
  counts: { meetings: 42, people: 5, topics: 3, actionItems: 20 },
  bytes: 48_000_000,
  localMeetings,
});

let settings = { ...baseSettings };
let overrides: Record<string, (args: unknown) => unknown> = {};

beforeEach(() => {
  settings = { ...baseSettings };
  overrides = {};
  Object.values(toast).forEach((f) => f.mockReset());
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string, args: unknown) => {
    if (overrides[cmd]) return overrides[cmd](args);
    if (cmd === 'api_backup_get_settings') return settings;
    if (cmd === 'api_backup_status') return null;
    if (cmd === 'api_backup_set_settings') {
      const a = args as { enabled: boolean; folder: string | null; includeAudio: boolean };
      settings = { ...settings, ...a };
      return settings;
    }
    return undefined;
  });
});

async function renderLoaded() {
  render(<BackupSection />);
  await waitFor(() =>
    expect(screen.getByRole('switch', { name: 'Back up automatically every day' })).not.toBeDisabled(),
  );
}

describe('BackupSection', () => {
  it('shows Never when there is no last backup', async () => {
    await renderLoaded();
    expect(screen.queryByText('Last backup')).not.toBeInTheDocument();
  });

  it('renders the last-backup line from settings', async () => {
    const at = new Date();
    at.setHours(9, 30, 0, 0);
    settings = {
      ...baseSettings,
      enabled: true,
      folder: '/Users/x/Backups',
      lastSuccessAt: at.toISOString(),
      lastSize: 48_000_000,
    };
    await renderLoaded();
    expect(screen.getByText('Today 09:30 · 48 MB')).toBeInTheDocument();
  });

  it('shows the last error on its own line', async () => {
    settings = { ...baseSettings, enabled: true, folder: '/b', lastError: 'The folder is full' };
    await renderLoaded();
    expect(screen.getByText('The folder is full')).toBeInTheDocument();
    expect(screen.getByText('Never')).toBeInTheDocument();
  });

  it('opens the folder picker when the schedule is switched on without a folder', async () => {
    overrides.api_backup_pick_folder = () => '/Users/x/Nixon Backups';
    await renderLoaded();
    fireEvent.click(screen.getByRole('switch', { name: 'Back up automatically every day' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_set_settings', {
        enabled: true,
        folder: '/Users/x/Nixon Backups',
        includeAudio: false,
      }),
    );
    expect(invoke).toHaveBeenCalledWith('api_backup_pick_folder');
  });

  it('stays off when the folder picker is cancelled', async () => {
    overrides.api_backup_pick_folder = () => null;
    await renderLoaded();
    fireEvent.click(screen.getByRole('switch', { name: 'Back up automatically every day' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_backup_pick_folder'));
    expect(invoke).not.toHaveBeenCalledWith('api_backup_set_settings', expect.anything());
    expect(screen.getByRole('switch', { name: 'Back up automatically every day' })).not.toBeChecked();
  });

  it('offers only Replace when this Mac has no meetings', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => previewOf(0);
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    expect(await screen.findByRole('button', { name: 'Replace everything…' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: "Add what's missing" })).not.toBeInTheDocument();
    expect(
      screen.getByText('Backup from Nov 24, 2026 · Nixon 0.18.2 · 42 meetings · without audio · 48 MB'),
    ).toBeInTheDocument();
  });

  it('offers both choices when this Mac has meetings', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => previewOf(7);
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    expect(await screen.findByRole('button', { name: "Add what's missing" })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Replace everything…' })).toBeInTheDocument();
  });

  it('shows the update copy for a backup from a newer Nixon', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => {
      throw { kind: 'newerSchema', message: 'This backup was made by a newer Nixon (0.19.0, schema 9).' };
    };
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'This backup is from a newer version of Nixon (0.19.0). Update Nixon, then import it.',
    );
  });

  it('says so when the file is not a Nixon backup', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => {
      throw { kind: 'notABackup', message: 'nope' };
    };
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    expect(await screen.findByRole('alert')).toHaveTextContent("This isn't a Nixon backup.");
  });

  it('toasts what Add brought in', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => previewOf(7);
    overrides.api_backup_import = () => ({
      meetings: 12,
      people: 3,
      topics: 2,
      actionItems: 9,
      alreadyHere: 30,
      newMeetingIds: [],
    });
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    fireEvent.click(await screen.findByRole('button', { name: "Add what's missing" }));
    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith(
        'Added 12 meetings, 3 people, 2 topics, 9 to-dos. 30 were already here.',
      ),
    );
    expect(invoke).toHaveBeenCalledWith('api_backup_import', { path: '/tmp/b.zip', mode: 'add' });
  });

  it('asks before replacing and then imports in replace mode', async () => {
    overrides.api_backup_pick_file = () => '/tmp/b.zip';
    overrides.api_backup_preview = () => previewOf(7);
    overrides.api_backup_import = () => null;
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Replace everything…' }));
    expect(await screen.findByText(/A copy of your current data is saved first\. Nixon restarts\./)).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('api_backup_import', expect.anything());
    fireEvent.click(screen.getByRole('button', { name: 'Replace everything' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_import', { path: '/tmp/b.zip', mode: 'replace' }),
    );
  });
});
