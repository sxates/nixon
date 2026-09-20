import { invoke } from '@tauri-apps/api/core';

/**
 * Patch the recording-preferences store: read what is stored, merge the change, write it
 * back (specs/0067).
 *
 * Every caller used to save its own whole `preferences` object, spread from state loaded
 * when that component mounted. With devices and recording behaviour on one screen that was
 * safe. specs/0067 moved the device pickers to the Audio tab, so two components now write
 * the same store — and a full-object save from a tab holding a stale copy would silently
 * revert the other tab's change. Read-modify-write removes that whole class of bug rather
 * than relying on both tabs staying in sync.
 *
 * Returns the merged object so callers can keep their optimistic local state honest.
 */
export async function patchRecordingPreferences<T extends object>(patch: Partial<T>): Promise<T> {
  const current = (await invoke<T>('get_recording_preferences')) ?? ({} as T);
  const merged = { ...current, ...patch };
  await invoke('set_recording_preferences', { preferences: merged });
  return merged;
}
