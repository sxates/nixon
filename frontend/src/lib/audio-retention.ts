/**
 * Audio retention (Settings → Recording → Audio storage, specs/0072).
 *
 * The UI shows ONE control, "Delete audio recordings": Once processed / after N days /
 * Never. It is stored as `audio_retention` in the recording preferences:
 *   `{mode:'after_processing'} | {mode:'days', days:N} | {mode:'forever'}`.
 *
 * Capture no longer reads this setting: every recording keeps its full audio until Rust's
 * lifecycle decides, per meeting, that processing (transcription + speaker
 * identification) has finished. Only then does the policy delete or compress it.
 *
 * The legacy pair `auto_save` / `retention_days` is still in the struct and still
 * mirrored here exactly as the backend's `AudioRetention::to_legacy` does, so a whole-
 * object save from any other Settings handler can never read as a policy change.
 */

import { formatBytes, plural } from '@/lib/recordings-move';

/** The stored policy (serde-tagged in Rust: `audio/lifecycle/policy.rs`). */
export type AudioRetention =
  | { mode: 'after_processing' }
  | { mode: 'days'; days: number }
  | { mode: 'forever' };

/** The single UI choice. */
export type AudioRetentionChoice = 'once-processed' | 'never' | number;

/** The stored fields the choice maps onto (subset of RecordingPreferences). */
export interface AudioRetentionFields {
  audio_retention?: AudioRetention | null;
  auto_save: boolean;
  retention_days: number | null;
}

/** `api_preview_audio_retention`: what the candidate policy would delete now. */
export interface RetentionPreview {
  meetings: number;
  bytes: number;
  /** Meetings the candidate would delete once they finish processing. */
  keptPending: number;
  /** Meetings whose speaker identification failed, inside their grace period. */
  keptFailed: number;
  /** Part of `meetings`: in use right now, deleted on the next pass. */
  busy: number;
}

/** `api_apply_audio_retention_now`: what the saved policy actually deleted. */
export interface RetentionReport {
  meetingsPurged: number;
  bytesFreed: number;
  skippedBusy: number;
}

const validDays = (n: unknown): n is number =>
  typeof n === 'number' && Number.isFinite(n) && n > 0;

/** Stored preferences → UI choice. The new field wins; the legacy pair is the fallback. */
export function retentionChoiceFromPreferences(
  prefs: AudioRetentionFields,
): AudioRetentionChoice {
  const policy = prefs.audio_retention;
  if (policy) {
    if (policy.mode === 'after_processing') return 'once-processed';
    if (policy.mode === 'days' && validDays(policy.days)) return Math.floor(policy.days);
    return 'never';
  }
  // Same derivation as Rust's `AudioRetention::from_legacy`.
  if (!prefs.auto_save) return 'once-processed';
  return validDays(prefs.retention_days) ? prefs.retention_days : 'never';
}

/** UI choice → stored policy. A non-positive day count degrades to "never" (keeps audio). */
export function retentionPolicyFromChoice(choice: AudioRetentionChoice): AudioRetention {
  if (choice === 'once-processed') return { mode: 'after_processing' };
  if (choice === 'never' || !validDays(choice)) return { mode: 'forever' };
  return { mode: 'days', days: Math.floor(choice) };
}

/**
 * UI choice → updated preferences: the policy plus the mirrored legacy pair. "Once
 * processed" keeps the stored day count so flipping back to a days option restores it.
 */
export function applyRetentionChoice<T extends AudioRetentionFields>(
  prefs: T,
  choice: AudioRetentionChoice,
): T {
  const audio_retention = retentionPolicyFromChoice(choice);
  switch (audio_retention.mode) {
    case 'after_processing':
      return { ...prefs, audio_retention, auto_save: false };
    case 'days':
      return { ...prefs, audio_retention, auto_save: true, retention_days: audio_retention.days };
    case 'forever':
      return { ...prefs, audio_retention, auto_save: true, retention_days: null };
  }
}

/** How much audio a choice keeps: higher keeps more. */
function retentionRank(choice: AudioRetentionChoice): number {
  if (choice === 'once-processed') return 0;
  if (choice === 'never') return Number.POSITIVE_INFINITY;
  return choice;
}

/** Does moving from `from` to `to` delete audio sooner? Only then is there anything to ask. */
export function isLoweringRetention(
  from: AudioRetentionChoice,
  to: AudioRetentionChoice,
): boolean {
  return retentionRank(to) < retentionRank(from);
}

/** Serialize a choice for the select ("once-processed" | "never" | "30"). */
export function retentionChoiceToSelectValue(choice: AudioRetentionChoice): string {
  return typeof choice === 'number' ? String(choice) : choice;
}

/** Parse a select value back into a choice. Unknown values → "never" (keeps audio). */
export function retentionChoiceFromSelectValue(value: string): AudioRetentionChoice {
  if (value === 'once-processed' || value === 'never') return value;
  const days = Number.parseInt(value, 10);
  return Number.isFinite(days) && days > 0 ? days : 'never';
}

/** What the selected option does, shown under the row label. Every line must be true. */
export function retentionChoiceDescription(choice: AudioRetentionChoice): string {
  if (choice === 'once-processed') {
    return (
      "Audio is kept until Nixon has transcribed the meeting and identified the speakers, then deleted. If you quit before that finishes, it's kept until processing completes. " +
      "If speaker identification fails, the audio is kept for 7 days so you can retry."
    );
  }
  if (choice === 'never') return 'Audio is kept, compressed, about 50 MB per hour of meeting.';
  return (
    `Audio from meetings older than ${choice} days is deleted, never while a meeting is still being transcribed or its speakers identified. ` +
    "Until then it's stored compressed, about 50 MB per hour."
  );
}

/** One-line statement of the rule, for the toast when nothing needs deleting now. */
export function retentionRuleSentence(choice: AudioRetentionChoice): string {
  if (choice === 'once-processed') {
    return 'Audio will be deleted once each meeting is transcribed and its speakers are identified.';
  }
  if (choice === 'never') return 'Audio will be kept.';
  return `Audio from meetings older than ${choice} days will be deleted.`;
}

/** The dialog's rule line: what the confirmed choice deletes now. */
export function retentionDeletesNowSentence(choice: AudioRetentionChoice): string {
  if (choice === 'once-processed') {
    return 'Audio from meetings that are already transcribed and have their speakers identified will be deleted.';
  }
  if (choice === 'never') return 'No audio will be deleted.';
  return `Audio from meetings older than ${choice} days will be deleted.`;
}

const meetingsText = (n: number) => plural(n, 'meeting');

/** The dialog's muted line about meetings that keep their audio for now, or null. */
export function keptForNowSentence(preview: RetentionPreview): string | null {
  const parts: string[] = [];
  if (preview.keptPending > 0) {
    parts.push(
      `${meetingsText(preview.keptPending)} still being processed ${preview.keptPending === 1 ? 'keeps its' : 'keep their'} audio until ${preview.keptPending === 1 ? 'it finishes' : 'they finish'}.`,
    );
  }
  if (preview.keptFailed > 0) {
    parts.push(
      `${meetingsText(preview.keptFailed)} whose speaker identification didn't finish ${preview.keptFailed === 1 ? 'keeps its' : 'keep their'} audio for now so you can retry.`,
    );
  }
  return parts.length ? parts.join(' ') : null;
}

/** The toast after "Delete audio": the numbers the backend reported, never the preview's. */
export function retentionReportToast(report: RetentionReport): {
  title: string;
  description?: string;
} {
  const busy =
    report.skippedBusy > 0
      ? `${report.skippedBusy} ${report.skippedBusy === 1 ? 'was' : 'were'} busy and will be deleted shortly.`
      : undefined;
  if (report.meetingsPurged === 0) {
    return { title: 'No audio was deleted yet', description: busy };
  }
  return {
    title: `Deleted audio from ${meetingsText(report.meetingsPurged)}, ${formatBytes(report.bytesFreed)} freed`,
    description: busy,
  };
}
