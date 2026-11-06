/**
 * specs/0083 W2b — the two preferences the Rust wrap-up reads: summarize automatically when a
 * meeting ends, and the pinned summary language. Rust owns them (`RecordingPreferences`, via
 * `api_get/set_wrap_up_preferences`) because the wrap-up runs with the window hidden. The
 * UI keeps its synchronous localStorage copies as a cache and writes through.
 *
 * They lived only in localStorage before, so the first launch after the upgrade pushes those
 * values to Rust once; every later launch pulls Rust's values into the cache instead.
 */

import { invoke } from '@tauri-apps/api/core';

export const AUTO_SUMMARY_KEY = 'isAutoSummary';
/** Same key as `summary-language-preferences.ts` SUMMARY_LANGUAGE_DEFAULT_KEY. */
const LANGUAGE_DEFAULT_KEY = 'summaryLanguageDefault';
/** Set once the localStorage values have been pushed to Rust. */
export const WRAP_UP_PREFS_MIGRATED_KEY = 'nixon.wrapUpPrefsMigrated';

export interface WrapUpPreferences {
  autoSummary: boolean;
  summaryLanguageDefault: string | null;
}

/** A partial update: an absent field is left alone; `summaryLanguageDefault: null` clears it. */
export type WrapUpPreferencesPatch = Partial<WrapUpPreferences>;

function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}

/** Write a change through to Rust. Best-effort: the local copy has already changed. */
export async function saveWrapUpPreferences(patch: WrapUpPreferencesPatch): Promise<void> {
  try {
    await invoke('api_set_wrap_up_preferences', { preferences: patch });
  } catch (error) {
    console.warn('[wrap-up prefs] could not save to Rust:', error);
  }
}

/**
 * Reconcile at launch. The first time, push what localStorage holds (only the keys the user
 * actually set) and mark it done; afterwards, pull Rust's values into localStorage. Returns
 * the values now in force, or null when Rust could not be reached (the cache stays as is).
 */
export async function syncWrapUpPreferences(): Promise<WrapUpPreferences | null> {
  const local = storage();
  try {
    if (local && local.getItem(WRAP_UP_PREFS_MIGRATED_KEY) !== 'true') {
      const patch: WrapUpPreferencesPatch = {};
      const auto = local.getItem(AUTO_SUMMARY_KEY);
      if (auto !== null) patch.autoSummary = auto === 'true';
      const language = local.getItem(LANGUAGE_DEFAULT_KEY);
      if (language) patch.summaryLanguageDefault = language;
      const push = (preferences: WrapUpPreferencesPatch) =>
        invoke<WrapUpPreferences>('api_set_wrap_up_preferences', { preferences });
      let saved: WrapUpPreferences;
      try {
        saved = await push(patch);
      } catch (error) {
        // A stored language Rust can't summarize in must not block the migration forever.
        if (!('summaryLanguageDefault' in patch)) throw error;
        console.warn('[wrap-up prefs] stored summary language refused; keeping Auto:', error);
        delete patch.summaryLanguageDefault;
        local.removeItem(LANGUAGE_DEFAULT_KEY);
        saved = await push(patch);
      }
      local.setItem(WRAP_UP_PREFS_MIGRATED_KEY, 'true');
      return saved;
    }
    const current = await invoke<WrapUpPreferences>('api_get_wrap_up_preferences');
    if (local && current) {
      local.setItem(AUTO_SUMMARY_KEY, String(current.autoSummary));
      if (current.summaryLanguageDefault) {
        local.setItem(LANGUAGE_DEFAULT_KEY, current.summaryLanguageDefault);
      } else {
        local.removeItem(LANGUAGE_DEFAULT_KEY);
      }
    }
    return current ?? null;
  } catch (error) {
    console.warn('[wrap-up prefs] could not sync with Rust:', error);
    return null;
  }
}
