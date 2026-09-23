/**
 * Moving the recordings folder (specs/0073) — the payload shapes the Rust mover sends
 * (`audio/recordings_move/`, camelCase JSON) plus the pure copy the UI builds from them.
 *
 * Every payload is shape-checked before use: `invoke` only promises the type it is told,
 * and a truthy `{}` from a mock or an older backend must read as "nothing is moving",
 * never as a move in progress.
 */

export type LeaseHolder =
  | 'recording'
  | 'retranscription'
  | 'diarization'
  | 'retention'
  | 'compression'
  | 'mover'
  | 'folderPathWrite'
  | 'delete'
  | 'discard';

export interface MovePlan {
  target: string;
  /** Distinct meetings that will move. */
  meetings: number;
  folders: number;
  bytes: number;
  sameVolumeCount: number;
  /** Folders copied across drives; > 0 means the move may take a while. */
  crossVolumeCount: number;
  crossVolumeBytes: number;
  /** Folders outside every folder Nixon knows about. They still move. */
  elsewhere: number;
  /** Meetings whose folder no longer exists. Not moved. */
  missing: number;
  notOwned: number;
  protected: number;
  unreferenced: number;
  freeBytes: number;
  enoughSpace: boolean;
}

export interface MoveStatus {
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  currentTitle: string | null;
  waitingFor: LeaseHolder | null;
  target: string;
}

export interface MoveFailure {
  meetingId: string;
  title: string;
  reason: string;
  /** Where the recording still is. */
  folderPath: string;
}

export interface MoveFinished {
  moved: number;
  skipped: number;
  failed: MoveFailure[];
  cancelled: boolean;
  removedRoots: string[];
}

export interface GatherState {
  needsConfirmation: boolean;
  blockedReason: string | null;
  plan: MovePlan;
}

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

export function isMovePlan(v: unknown): v is MovePlan {
  return (
    isObject(v) &&
    typeof v.target === 'string' &&
    typeof v.meetings === 'number' &&
    typeof v.bytes === 'number' &&
    typeof v.enoughSpace === 'boolean'
  );
}

export function isMoveStatus(v: unknown): v is MoveStatus {
  return isObject(v) && typeof v.done === 'number' && typeof v.total === 'number';
}

export function isMoveFinished(v: unknown): v is MoveFinished {
  return isObject(v) && typeof v.moved === 'number' && Array.isArray(v.failed);
}

export function isGatherState(v: unknown): v is GatherState {
  return isObject(v) && typeof v.needsConfirmation === 'boolean' && isMovePlan(v.plan);
}

/** The backend's refusal text, as-is. Rust commands reject with a plain string. */
export function errorText(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Decimal units, as Finder shows them. */
export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return bytes > 0 ? 'less than 1 MB' : '0 MB';
}

export const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

const WAITING_COPY: Record<LeaseHolder, string> = {
  recording: 'Waiting for the recording to finish…',
  retranscription: 'Waiting for transcription to finish…',
  diarization: 'Waiting for speaker identification to finish…',
  retention: 'Waiting for audio cleanup to finish…',
  compression: 'Waiting for audio compression to finish…',
  mover: 'Waiting for another move to finish…',
  folderPathWrite: 'Waiting for the transcript to save…',
  delete: 'Waiting for a meeting to finish deleting…',
  discard: 'Waiting for a recording to be discarded…',
};

/** The progress line under the Save location row. */
export function progressLine(status: MoveStatus): string {
  if (status.waitingFor) return WAITING_COPY[status.waitingFor] ?? 'Waiting…';
  const n = Math.min(status.done + 1, Math.max(status.total, 1));
  const head = `Moving ${n} of ${status.total}`;
  return status.currentTitle ? `${head} — ${status.currentTitle}` : head;
}

/** 0–100. Bytes when there are any to copy (cross-drive), else meetings. */
export function progressPercent(status: MoveStatus): number {
  const ratio =
    status.bytesTotal > 0
      ? status.bytesDone / status.bytesTotal
      : status.total > 0
        ? status.done / status.total
        : 0;
  return Math.max(0, Math.min(100, Math.round(ratio * 100)));
}

export interface FinishToast {
  kind: 'success' | 'info' | 'warning';
  title: string;
  description?: string;
  /** The failure whose folder "Show" opens. */
  show?: MoveFailure;
}

/** What the app-level toast says when a move ends. `null` = nothing worth saying. */
export function finishToast(f: MoveFinished): FinishToast | null {
  if (f.failed.length > 0) {
    const first = f.failed[0];
    return {
      kind: 'warning',
      title: `Moved ${f.moved} of ${f.moved + f.failed.length} — ${f.failed.length} couldn't be moved`,
      description: first.title ? `${first.title}: ${first.reason}` : first.reason,
      show: first,
    };
  }
  if (f.cancelled) {
    return {
      kind: 'info',
      title: `Stopped after moving ${plural(f.moved, 'recording')}`,
      description: 'The rest stay where they are. Settings → Recording can move them later.',
    };
  }
  if (f.moved > 0) return { kind: 'success', title: `Moved ${plural(f.moved, 'recording')}` };
  return null;
}

/** iCloud Drive is allowed but can evict local copies (spec 0073 open question 4). */
export function isICloudDrivePath(path: string): boolean {
  return /^\/(?:Users|home)\/[^/]+\/Library\/Mobile Documents(?:\/|$)/.test(path);
}
