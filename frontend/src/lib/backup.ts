/**
 * Backup and restore (specs/0092): the TS side of the `api_backup_*` commands in
 * `src-tauri/src/backup/commands.rs`. Every shape is camelCase on the wire.
 */

import { formatBytes, plural } from '@/lib/recordings-move';

export const BACKUP_PROGRESS_EVENT = 'backup-progress';
export const BACKUP_FINISHED_EVENT = 'backup-finished';

export type BackupJobKind = 'export' | 'import' | 'scheduled';

export interface BackupSettings {
  enabled: boolean;
  folder: string | null;
  includeAudio: boolean;
  lastSuccessAt: string | null;
  lastError: string | null;
  lastSize: number | null;
}

export interface BackupProgress {
  kind: BackupJobKind;
  phase: 'snapshot' | 'writing' | 'extracting' | 'applying' | null;
  done: number;
  total: number;
}

export interface BackupFinished {
  kind: BackupJobKind;
  ok: boolean;
  path?: string;
  error?: string;
}

export interface BackupPreview {
  createdAt: string;
  appVersion: string;
  includesAudio: boolean;
  counts: { meetings: number; people: number; topics: number; actionItems: number };
  bytes: number;
  localMeetings: number;
}

export interface MergeCounts {
  meetings: number;
  people: number;
  topics: number;
  actionItems: number;
  alreadyHere: number;
}

export interface RunNowResult {
  result: 'written' | 'skipped' | 'notDue';
  path?: string;
  bytes?: number;
}

export interface RestoreResult {
  ok: boolean;
  meetings: number;
  createdAt: string;
  error?: string;
}

/** The `{kind, message}` a failed backup command rejects with. */
export function backupErrorKind(error: unknown): string | null {
  if (error && typeof error === 'object' && 'kind' in error) {
    const kind = (error as { kind: unknown }).kind;
    return typeof kind === 'string' ? kind : null;
  }
  return null;
}

export function backupErrorText(error: unknown): string {
  const kind = backupErrorKind(error);
  const message =
    error && typeof error === 'object' && 'message' in error
      ? String((error as { message: unknown }).message)
      : typeof error === 'string'
        ? error
        : '';
  switch (kind) {
    case 'newerSchema': {
      // The Rust message reads "… newer Nixon (0.19.0, schema 9) …"; lift the version out.
      const version = /newer Nixon \(([^,)]+)/.exec(message)?.[1];
      return `This backup is from a newer version of Nixon${version ? ` (${version})` : ''}. Update Nixon, then import it.`;
    }
    case 'notABackup':
      return "This isn't a Nixon backup.";
    case 'busy':
      return 'A backup is already running.';
    default:
      return message || 'Something went wrong with the backup.';
  }
}

export const isCancelled = (error: unknown) => backupErrorKind(error) === 'cancelled';

const pad2 = (n: number) => String(n).padStart(2, '0');

/** "Nov 24, 2026" — for the import preview and the Replace confirm. */
export function formatBackupDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

/** "Today 09:30 · 48 MB", "Yesterday 21:10 · 48 MB", "Nov 20 · 48 MB", or "Never". */
export function formatLastBackup(
  at: string | null,
  size: number | null,
  now: Date = new Date(),
): string {
  if (!at) return 'Never';
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return 'Never';
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const time = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  let when: string;
  if (sameDay(d, now)) when = `Today ${time}`;
  else if (sameDay(d, yesterday)) when = `Yesterday ${time}`;
  else {
    when = d.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      ...(d.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
    });
  }
  return size ? `${when} · ${formatBytes(size)}` : when;
}

/** "Backup from Nov 24, 2026 · Nixon 0.18.2 · 42 meetings · without audio · 48 MB". */
export function previewLine(p: BackupPreview): string {
  return [
    `Backup from ${formatBackupDate(p.createdAt)}`,
    `Nixon ${p.appVersion}`,
    plural(p.counts.meetings, 'meeting'),
    p.includesAudio ? 'with audio' : 'without audio',
    formatBytes(p.bytes),
  ].join(' · ');
}

/** The toast after "Add what's missing". */
export function addResultText(c: MergeCounts): string {
  const parts = [
    c.meetings > 0 && plural(c.meetings, 'meeting'),
    c.people > 0 && plural(c.people, 'person', 'people'),
    c.topics > 0 && plural(c.topics, 'topic'),
    c.actionItems > 0 && plural(c.actionItems, 'to-do'),
  ].filter(Boolean);
  if (parts.length === 0) return 'Nothing new to add. Everything in this backup was already here.';
  const here = c.alreadyHere > 0 ? ` ${c.alreadyHere} ${c.alreadyHere === 1 ? 'was' : 'were'} already here.` : '';
  return `Added ${parts.join(', ')}.${here}`;
}

/** The once-per-launch toast after a Replace restart, or null when nothing was restored. */
export function restoreToast(r: RestoreResult): { ok: boolean; text: string } {
  if (r.ok) {
    return {
      ok: true,
      text: `Restored ${plural(r.meetings, 'meeting')} from ${formatBackupDate(r.createdAt)}.`,
    };
  }
  return { ok: false, text: `Restore failed: ${r.error ?? 'Unknown error.'}` };
}
