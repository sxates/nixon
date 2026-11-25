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
  folder: '/Users/x/Movies/nixon-recordings',
  includeAudio: false,
  lastSuccessAt: null as string | null,
  lastError: null as string | null,
  lastSize: null as number | null,
};

const today = (h: number, m: number) => {
  const d = new Date();
  d.setHours(h, m, 0, 0);
  return d.toISOString();
};

const backupFiles = () => [
  {
    path: '/Users/x/Movies/nixon-recordings/Nixon Auto Backup 2026-11-24 0930.zip',
    name: 'Nixon Auto Backup 2026-11-24 0930.zip',
    auto: true,
    bytes: 48_000_000,
    modifiedAt: today(9, 30),
  },
  {
    path: '/Users/x/Movies/nixon-recordings/Nixon Backup 2026-11-24 0800.zip',
    name: 'Nixon Backup 2026-11-24 0800.zip',
    auto: false,
    bytes: 47_000_000,
    modifiedAt: today(8, 0),
  },
];

const previewOf = (localMeetings: number) => ({
  createdAt: '2026-11-24T10:00:00Z',
  appVersion: '0.18.2',
  includesAudio: false,
  counts: { meetings: 42, people: 5, topics: 3, actionItems: 20 },
  bytes: 48_000_000,
  localMeetings,
});

let settings = { ...baseSettings };
let files = backupFiles();
let overrides: Record<string, (args: unknown) => unknown> = {};

beforeEach(() => {
  settings = { ...baseSettings };
  files = backupFiles();
  overrides = {};
  Object.values(toast).forEach((f) => f.mockReset());
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string, args: unknown) => {
    if (overrides[cmd]) return overrides[cmd](args);
    if (cmd === 'api_backup_get_settings') return settings;
    if (cmd === 'api_backup_status') return null;
    if (cmd === 'api_backup_list') return files;
    if (cmd === 'api_backup_delete') {
      const { path } = args as { path: string };
      files = files.filter((f) => f.path !== path);
      return null;
    }
    if (cmd === 'api_backup_set_settings') {
      const a = args as { enabled: boolean; includeAudio: boolean };
      settings = { ...settings, ...a };
      return settings;
    }
    if (cmd === 'api_backup_set_folder') {
      settings = { ...settings, folder: (args as { folder: string }).folder };
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
  it('always shows the backup folder, defaulting to the recordings folder', async () => {
    await renderLoaded();
    expect(screen.getByText('Backup folder')).toBeInTheDocument();
    expect(screen.getByText('~/Movies/nixon-recordings')).toBeInTheDocument();
  });

  it('changes the folder and reloads the list', async () => {
    overrides.api_backup_pick_folder = () => '/Users/x/Nixon Backups';
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_set_folder', { folder: '/Users/x/Nixon Backups' }),
    );
    expect(await screen.findByText('~/Nixon Backups')).toBeInTheDocument();
  });

  it('lists manual and automatic backups, newest first', async () => {
    await renderLoaded();
    const list = await screen.findByRole('list', { name: 'Backups' });
    const items = list.querySelectorAll('li');
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent('Today 09:30 · 48 MB');
    expect(items[0]).toHaveTextContent('Automatic');
    expect(items[1]).toHaveTextContent('Today 08:00 · 47 MB');
    expect(items[1]).toHaveTextContent('Manual');
  });

  it('says so when the folder has no backups', async () => {
    files = [];
    await renderLoaded();
    expect(await screen.findByText('No backups in this folder yet.')).toBeInTheDocument();
  });

  it('deletes a backup after confirming', async () => {
    await renderLoaded();
    await screen.findByRole('list', { name: 'Backups' });
    fireEvent.click(screen.getAllByRole('button', { name: 'Delete backup' })[1]);
    expect(
      await screen.findByText(
        "Nixon Backup 2026-11-24 0800.zip is deleted from the backup folder. This can't be undone.",
      ),
    ).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('api_backup_delete', expect.anything());
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_delete', {
        path: '/Users/x/Movies/nixon-recordings/Nixon Backup 2026-11-24 0800.zip',
      }),
    );
    await waitFor(() => expect(screen.getAllByRole('button', { name: 'Delete backup' })).toHaveLength(1));
  });

  it('shows a backup in Finder', async () => {
    await renderLoaded();
    await screen.findByRole('list', { name: 'Backups' });
    fireEvent.click(screen.getAllByRole('button', { name: 'Show in Finder' })[0]);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_reveal', {
        path: '/Users/x/Movies/nixon-recordings/Nixon Auto Backup 2026-11-24 0930.zip',
      }),
    );
  });

  it('one switch turns on daily backups and says how many it keeps', async () => {
    await renderLoaded();
    expect(
      screen.getByText(
        'Keeps the 5 most recent automatic backups. Backups you make yourself are kept until you delete them.',
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('switch', { name: 'Back up automatically every day' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_set_settings', { enabled: true, includeAudio: false }),
    );
    expect(invoke).not.toHaveBeenCalledWith('api_backup_pick_folder');
  });

  it('one switch covers audio for every backup', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('switch', { name: 'Include audio' }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_backup_set_settings', { enabled: false, includeAudio: true }),
    );
  });

  it('Back up now writes into the folder and refreshes the list', async () => {
    overrides.api_backup_export = () => {
      files = [
        {
          path: '/Users/x/Movies/nixon-recordings/Nixon Backup 2026-11-24 1000.zip',
          name: 'Nixon Backup 2026-11-24 1000.zip',
          auto: false,
          bytes: 49_000_000,
          modifiedAt: today(10, 0),
        },
        ...files,
      ];
      return files[0].path;
    };
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: 'Back up now' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_backup_export'));
    await waitFor(() => expect(screen.getAllByRole('button', { name: 'Delete backup' })).toHaveLength(3));
    expect(toast.success).toHaveBeenCalledWith('Backup saved', expect.anything());
  });

  it('shows the last automatic-backup error', async () => {
    settings = { ...baseSettings, enabled: true, lastError: 'The folder is full' };
    await renderLoaded();
    expect(screen.getByText('The folder is full')).toBeInTheDocument();
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
