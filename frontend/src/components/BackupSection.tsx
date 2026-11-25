'use client';

/**
 * Settings > General > Backup (specs/0092). Every backup goes into one folder (the recordings
 * folder unless another is chosen) and is listed here, newest first, with a delete button.
 * Backups made with "Back up now" are kept until deleted; the daily schedule keeps its newest
 * five. One Include audio switch covers both. Import sits below, as the secondary action.
 *
 * "Back up now" shows a determinate bar with Cancel; a scheduled run reports no byte progress
 * and can't be cancelled, so it shows an indeterminate "Backing up…" line instead.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { FolderOpen, Trash2 } from '@/components/ui/icons';
import { Progress } from '@/components/ui/progress';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';
import { Spinner } from '@/components/ui/spinner';
import { Switch } from '@/components/ui/switch';
import { BackupImportDialog } from '@/components/BackupImportDialog';
import { tildePath } from '@/lib/format-path';
import { safeListen } from '@/lib/safe-listen';
import {
  AUTO_BACKUPS_KEPT,
  BACKUP_FINISHED_EVENT,
  BACKUP_PROGRESS_EVENT,
  addResultText,
  backupErrorText,
  formatLastBackup,
  isCancelled,
  type BackupFile,
  type BackupFinished,
  type BackupPreview,
  type BackupProgress,
  type BackupSettings,
  type MergeCounts,
} from '@/lib/backup';

const PHASE_LABEL: Record<NonNullable<BackupProgress['phase']>, string> = {
  snapshot: 'Preparing…',
  writing: 'Writing the backup…',
  extracting: 'Reading the backup…',
  applying: 'Applying…',
};

function progressLine(p: BackupProgress): string {
  if (p.phase) return PHASE_LABEL[p.phase];
  return p.kind === 'import' ? 'Importing…' : 'Backing up…';
}

export function BackupSection() {
  const [settings, setSettings] = useState<BackupSettings | null>(null);
  const [backups, setBackups] = useState<BackupFile[]>([]);
  const [progress, setProgress] = useState<BackupProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<{ path: string; data: BackupPreview } | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [toDelete, setToDelete] = useState<BackupFile | null>(null);
  const [deleting, setDeleting] = useState(false);

  const loadBackups = useCallback(async () => {
    try {
      setBackups((await invoke<BackupFile[]>('api_backup_list')) ?? []);
    } catch (error) {
      console.error('Failed to list backups:', error);
    }
  }, []);

  const loadSettings = useCallback(async () => {
    try {
      setSettings(await invoke<BackupSettings>('api_backup_get_settings'));
    } catch (error) {
      console.error('Failed to load backup settings:', error);
    }
  }, []);

  useEffect(() => {
    void loadSettings();
    void loadBackups();
    // Re-attach to a job that started before Settings opened.
    invoke<BackupProgress | null>('api_backup_status')
      .then((s) => s && setProgress(s))
      .catch(() => {});
    const offs = [
      safeListen<BackupProgress>(BACKUP_PROGRESS_EVENT, (e) => setProgress(e.payload)),
      safeListen<BackupFinished>(BACKUP_FINISHED_EVENT, () => {
        setProgress(null);
        void loadSettings();
        void loadBackups();
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [loadSettings, loadBackups]);

  const saveSwitches = async (next: { enabled: boolean; includeAudio: boolean }) => {
    try {
      setSettings(await invoke<BackupSettings>('api_backup_set_settings', next));
    } catch (error) {
      toast.error(backupErrorText(error));
    }
  };

  const onChangeFolder = async () => {
    try {
      const folder = await invoke<string | null>('api_backup_pick_folder');
      if (!folder) return;
      setSettings(await invoke<BackupSettings>('api_backup_set_folder', { folder }));
      void loadBackups();
    } catch (error) {
      toast.error(backupErrorText(error));
    }
  };

  const onBackUpNow = async () => {
    setBusy(true);
    try {
      const path = await invoke<string | null>('api_backup_export');
      if (path) toast.success('Backup saved', { description: tildePath(path) });
    } catch (error) {
      if (isCancelled(error)) toast.info('Backup cancelled');
      else toast.error(backupErrorText(error));
    } finally {
      setBusy(false);
      void loadBackups();
    }
  };

  const onDelete = async () => {
    if (!toDelete) return;
    setDeleting(true);
    try {
      await invoke('api_backup_delete', { path: toDelete.path });
      setToDelete(null);
    } catch (error) {
      toast.error(backupErrorText(error));
    } finally {
      setDeleting(false);
      void loadBackups();
    }
  };

  const onImportClick = async () => {
    setImportError(null);
    setBusy(true);
    try {
      const path = await invoke<string | null>('api_backup_pick_file');
      if (!path) return;
      const data = await invoke<BackupPreview>('api_backup_preview', { path });
      setPreview({ path, data });
    } catch (error) {
      setImportError(backupErrorText(error));
    } finally {
      setBusy(false);
    }
  };

  const runImport = async (mode: 'add' | 'replace') => {
    if (!preview) return;
    const { path } = preview;
    setPreview(null);
    setBusy(true);
    try {
      const counts = await invoke<MergeCounts | null>('api_backup_import', { path, mode });
      if (counts) toast.success(addResultText(counts));
    } catch (error) {
      if (isCancelled(error)) toast.info('Import cancelled');
      else toast.error(backupErrorText(error));
    } finally {
      setBusy(false);
    }
  };

  const onCancel = () => {
    void invoke('api_backup_cancel').catch((error) => console.error('Failed to cancel backup:', error));
  };

  const manual = progress && progress.kind !== 'scheduled' ? progress : null;
  const scheduledRunning = progress?.kind === 'scheduled';
  const working = busy || !!progress;

  return (
    <SettingsSection
      id="backup"
      title="Backup"
      description="Export all data from Nixon for a backup, or to move data to a new computer."
    >
      <SettingsGroup>
        <SettingsRow
          label="Backup folder"
          description={
            <span className="break-all" title={settings?.folder}>
              {settings ? tildePath(settings.folder) : '…'}
            </span>
          }
          control={
            <Button variant="outline" size="sm" onClick={onChangeFolder} disabled={!settings || working}>
              Change…
            </Button>
          }
        >
          <p className="text-xs text-muted-foreground">
            Backups aren&rsquo;t encrypted. A synced folder stores them with that service.
          </p>
        </SettingsRow>
        <SettingsRow
          label="Include audio"
          description="Recordings make backups much larger. Transcripts and notes are always included."
          control={
            <Switch
              checked={settings?.includeAudio ?? false}
              disabled={!settings || working}
              onCheckedChange={(includeAudio) =>
                settings && saveSwitches({ enabled: settings.enabled, includeAudio })
              }
              aria-label="Include audio"
            />
          }
        />
        <SettingsRow
          label="Back up automatically every day"
          description={`Keeps the ${AUTO_BACKUPS_KEPT} most recent automatic backups. Backups you make yourself are kept until you delete them.`}
          control={
            <Switch
              checked={settings?.enabled ?? false}
              disabled={!settings || working}
              onCheckedChange={(enabled) =>
                settings && saveSwitches({ enabled, includeAudio: settings.includeAudio })
              }
              aria-label="Back up automatically every day"
            />
          }
        >
          {settings?.enabled && settings.lastError ? (
            <Callout tone="warn" size="compact">
              {settings.lastError}
            </Callout>
          ) : null}
        </SettingsRow>
        <SettingsRow
          label="Backups"
          control={
            <Button variant="outline" size="sm" onClick={onBackUpNow} disabled={!settings || working}>
              Back up now
            </Button>
          }
        >
          {manual && manual.kind === 'export' ? (
            <ProgressLine progress={manual} onCancel={onCancel} />
          ) : scheduledRunning ? (
            <p className="flex items-center gap-2 text-xs text-muted-foreground" aria-live="polite">
              <Spinner /> Backing up…
            </p>
          ) : null}
          {backups.length === 0 ? (
            <p className="text-sm text-muted-foreground">No backups in this folder yet.</p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border" aria-label="Backups">
              {backups.map((b) => (
                <li key={b.path} className="flex items-center gap-3 px-3 py-2">
                  <span className="min-w-0 flex-1 truncate text-sm" title={b.name}>
                    {formatLastBackup(b.modifiedAt, b.bytes)}
                  </span>
                  <span className="flex-none text-xs text-muted-foreground">
                    {b.auto ? 'Automatic' : 'Manual'}
                  </span>
                  <IconButton
                    variant="ghost"
                    label="Show in Finder"
                    icon={<DeckIcon icon={FolderOpen} />}
                    onClick={() =>
                      void invoke('api_backup_reveal', { path: b.path }).catch((error) =>
                        toast.error(backupErrorText(error)),
                      )
                    }
                  />
                  <IconButton
                    variant="ghost"
                    label="Delete backup"
                    icon={<DeckIcon icon={Trash2} />}
                    className="hover:text-destructive"
                    disabled={working}
                    onClick={() => setToDelete(b)}
                  />
                </li>
              ))}
            </ul>
          )}
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup>
        <SettingsRow
          label="Import a backup"
          description="Add what's missing from a backup, or replace everything on this computer with it."
          control={
            <Button variant="outline" size="sm" onClick={onImportClick} disabled={working}>
              Import…
            </Button>
          }
        >
          {manual && manual.kind === 'import' ? <ProgressLine progress={manual} onCancel={onCancel} /> : null}
          {importError ? <Callout tone="error">{importError}</Callout> : null}
        </SettingsRow>
      </SettingsGroup>

      <ConfirmDialog
        open={!!toDelete}
        onOpenChange={(open) => !open && setToDelete(null)}
        title="Delete this backup?"
        body={toDelete ? `${toDelete.name} is deleted from the backup folder. This can't be undone.` : null}
        confirmLabel="Delete"
        busy={deleting}
        onConfirm={onDelete}
      />

      <BackupImportDialog
        preview={preview?.data ?? null}
        onClose={() => setPreview(null)}
        onAdd={() => void runImport('add')}
        onReplace={() => void runImport('replace')}
      />
    </SettingsSection>
  );
}

function ProgressLine({ progress, onCancel }: { progress: BackupProgress; onCancel: () => void }) {
  return (
    <div className="mb-3 space-y-2" data-testid="backup-progress">
      <Progress
        value={progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : 0}
        className="h-1.5"
      />
      <div className="flex items-center justify-between gap-4">
        <p className="min-w-0 truncate text-xs text-muted-foreground" aria-live="polite">
          {progressLine(progress)}
        </p>
        <Button variant="outline" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
