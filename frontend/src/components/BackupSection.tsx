'use client';

/**
 * Settings > General > Backup (specs/0092). Export or import a backup by hand, or let Nixon
 * write one into a folder every day. Manual export/import show a determinate bar with Cancel;
 * a scheduled run (or "Back up now") reports no byte progress and can't be cancelled, so it
 * shows an indeterminate "Backing up…" line instead.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { Checkbox } from '@/components/ui/checkbox';
import { Progress } from '@/components/ui/progress';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';
import { Spinner } from '@/components/ui/spinner';
import { Switch } from '@/components/ui/switch';
import { BackupImportDialog } from '@/components/BackupImportDialog';
import { tildePath } from '@/lib/format-path';
import { safeListen } from '@/lib/safe-listen';
import {
  BACKUP_FINISHED_EVENT,
  BACKUP_PROGRESS_EVENT,
  addResultText,
  backupErrorText,
  formatLastBackup,
  isCancelled,
  type BackupFinished,
  type BackupPreview,
  type BackupProgress,
  type BackupSettings,
  type MergeCounts,
  type RunNowResult,
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
  const [progress, setProgress] = useState<BackupProgress | null>(null);
  const [includeAudioOnce, setIncludeAudioOnce] = useState(false);
  const [busy, setBusy] = useState(false);
  const [runningNow, setRunningNow] = useState(false);
  const [preview, setPreview] = useState<{ path: string; data: BackupPreview } | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const loadSettings = useCallback(async () => {
    try {
      setSettings(await invoke<BackupSettings>('api_backup_get_settings'));
    } catch (error) {
      console.error('Failed to load backup settings:', error);
    }
  }, []);

  useEffect(() => {
    void loadSettings();
    // Re-attach to a job that started before Settings opened.
    invoke<BackupProgress | null>('api_backup_status')
      .then((s) => s && setProgress(s))
      .catch(() => {});
    const offs = [
      safeListen<BackupProgress>(BACKUP_PROGRESS_EVENT, (e) => setProgress(e.payload)),
      safeListen<BackupFinished>(BACKUP_FINISHED_EVENT, () => {
        setProgress(null);
        void loadSettings();
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [loadSettings]);

  const save = async (next: { enabled: boolean; folder: string | null; includeAudio: boolean }) => {
    try {
      setSettings(await invoke<BackupSettings>('api_backup_set_settings', next));
    } catch (error) {
      toast.error(backupErrorText(error));
    }
  };

  const pickFolder = async (): Promise<string | null> => {
    try {
      return (await invoke<string | null>('api_backup_pick_folder')) ?? null;
    } catch (error) {
      toast.error(backupErrorText(error));
      return null;
    }
  };

  const onToggleSchedule = async (on: boolean) => {
    if (!settings) return;
    let folder = settings.folder;
    if (on && !folder) {
      folder = await pickFolder();
      if (!folder) return;
    }
    await save({ enabled: on, folder, includeAudio: settings.includeAudio });
  };

  const onChangeFolder = async () => {
    if (!settings) return;
    const folder = await pickFolder();
    if (folder) await save({ enabled: settings.enabled, folder, includeAudio: settings.includeAudio });
  };

  const onExport = async () => {
    setBusy(true);
    try {
      const path = await invoke<string | null>('api_backup_export', { includeAudio: includeAudioOnce });
      if (path) toast.success('Backup saved', { description: tildePath(path) });
    } catch (error) {
      if (isCancelled(error)) toast.info('Backup cancelled');
      else toast.error(backupErrorText(error));
    } finally {
      setBusy(false);
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

  const onBackUpNow = async () => {
    setRunningNow(true);
    try {
      const r = await invoke<RunNowResult>('api_backup_run_now');
      if (r.result === 'written') toast.success('Backup saved');
      else toast.info('Nothing has changed since your last backup.');
    } catch (error) {
      toast.error(backupErrorText(error));
    } finally {
      setRunningNow(false);
      void loadSettings();
    }
  };

  const onCancel = () => {
    void invoke('api_backup_cancel').catch((error) => console.error('Failed to cancel backup:', error));
  };

  const enabled = settings?.enabled ?? false;
  const manual = progress && progress.kind !== 'scheduled' ? progress : null;
  const scheduledRunning = runningNow || progress?.kind === 'scheduled';
  const working = busy || runningNow || !!progress;

  return (
    <SettingsSection
      id="backup"
      title="Backup"
      description="Keep a copy of your meetings, notes, To-Dos and Topics, so you can move to a new Mac or recover from a dead one."
    >
      <SettingsGroup>
        <SettingsRow
          label="Export a backup"
          control={
            <div className="flex items-center gap-4">
              <Checkbox
                label="Include audio"
                checked={includeAudioOnce}
                onCheckedChange={setIncludeAudioOnce}
                disabled={working}
              />
              <Button variant="outline" size="sm" onClick={onExport} disabled={working}>
                Export…
              </Button>
            </div>
          }
        />
        <SettingsRow
          label="Import a backup"
          control={
            <Button variant="outline" size="sm" onClick={onImportClick} disabled={working}>
              Import…
            </Button>
          }
        >
          {importError ? <Callout tone="error">{importError}</Callout> : null}
        </SettingsRow>
        <SettingsRow
          label="Back up automatically every day"
          control={
            <Switch
              checked={enabled}
              disabled={!settings || working}
              onCheckedChange={onToggleSchedule}
              aria-label="Back up automatically every day"
            />
          }
        />
        {enabled && settings ? (
          <>
            <SettingsRow
              label="Folder"
              description={
                <span className="break-all" title={settings.folder ?? undefined}>
                  {settings.folder ? tildePath(settings.folder) : 'No folder chosen'}
                </span>
              }
              control={
                <Button variant="outline" size="sm" onClick={onChangeFolder} disabled={working}>
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
              control={
                <Switch
                  checked={settings.includeAudio}
                  disabled={working}
                  onCheckedChange={(includeAudio) =>
                    save({ enabled: settings.enabled, folder: settings.folder, includeAudio })
                  }
                  aria-label="Include audio in automatic backups"
                />
              }
            />
            <SettingsRow
              label="Last backup"
              description={formatLastBackup(settings.lastSuccessAt, settings.lastSize)}
              control={
                <Button variant="outline" size="sm" onClick={onBackUpNow} disabled={working}>
                  Back up now
                </Button>
              }
            >
              {settings.lastError ? (
                <Callout tone="warn" size="compact">
                  {settings.lastError}
                </Callout>
              ) : null}
            </SettingsRow>
          </>
        ) : null}
      </SettingsGroup>

      {manual ? (
        <div className="space-y-2" data-testid="backup-progress">
          <Progress
            value={manual.total > 0 ? Math.min(100, Math.round((manual.done / manual.total) * 100)) : 0}
            className="h-1.5"
          />
          <div className="flex items-center justify-between gap-4">
            <p className="min-w-0 truncate text-xs text-muted-foreground" aria-live="polite">
              {progressLine(manual)}
            </p>
            <Button variant="outline" size="sm" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        </div>
      ) : scheduledRunning ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground" aria-live="polite">
          <Spinner /> Backing up…
        </p>
      ) : null}

      <BackupImportDialog
        preview={preview?.data ?? null}
        onClose={() => setPreview(null)}
        onAdd={() => void runImport('add')}
        onReplace={() => void runImport('replace')}
      />
    </SettingsSection>
  );
}
