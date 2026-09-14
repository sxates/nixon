/**
 * Audio-retention mapping (Settings → Recordings).
 *
 * The UI shows ONE control — "Delete audio recordings": Immediately / after N
 * days / Never — but the backend contract is unchanged and still stores TWO
 * values in the recording preferences:
 *   - `auto_save`       (bool)          — save the audio file when recording stops
 *   - `retention_days`  (number | null) — sweep audio older than N days; null = keep
 *
 * Mapping rules (both directions):
 *   - Immediately  ⇔ auto_save=false (audio is discarded when recording stops).
 *     Writing "Immediately" leaves `retention_days` untouched, so flipping back
 *     to a days option restores the previous choice. Reading auto_save=false is
 *     ALWAYS "Immediately", regardless of any stored retention_days.
 *   - Never        ⇔ auto_save=true + retention_days=null.
 *   - N days       ⇔ auto_save=true + retention_days=N (N > 0).
 */

/** The single UI choice: discard at stop, keep N days, or keep forever. */
export type AudioRetentionChoice = 'immediately' | 'never' | number;

/** The two stored fields the choice maps onto (subset of RecordingPreferences). */
export interface AudioRetentionFields {
  auto_save: boolean;
  retention_days: number | null;
}

/** Stored preferences → UI choice. auto_save=false wins over any stored retention. */
export function retentionChoiceFromPreferences(
  prefs: AudioRetentionFields,
): AudioRetentionChoice {
  if (!prefs.auto_save) return 'immediately';
  return prefs.retention_days != null && prefs.retention_days > 0
    ? prefs.retention_days
    : 'never';
}

/**
 * UI choice → updated preferences. Non-destructive on "immediately": the stored
 * retention_days is preserved so it can be restored later.
 */
export function applyRetentionChoice<T extends AudioRetentionFields>(
  prefs: T,
  choice: AudioRetentionChoice,
): T {
  if (choice === 'immediately') return { ...prefs, auto_save: false };
  if (choice === 'never') return { ...prefs, auto_save: true, retention_days: null };
  // Defensive: a non-positive/NaN day count degrades to "never" rather than
  // writing an invalid retention the backend sweep would ignore anyway.
  const days = Number.isFinite(choice) && choice > 0 ? Math.floor(choice) : null;
  return { ...prefs, auto_save: true, retention_days: days };
}

/** Serialize a choice for a `<select>` value ("immediately" | "never" | "30"). */
export function retentionChoiceToSelectValue(choice: AudioRetentionChoice): string {
  return typeof choice === 'number' ? String(choice) : choice;
}

/** Parse a `<select>` value back into a choice. Unknown values → "never". */
export function retentionChoiceFromSelectValue(value: string): AudioRetentionChoice {
  if (value === 'immediately' || value === 'never') return value;
  const days = Number.parseInt(value, 10);
  return Number.isFinite(days) && days > 0 ? days : 'never';
}
