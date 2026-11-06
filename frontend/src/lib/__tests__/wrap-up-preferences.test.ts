import { describe, it, expect, beforeEach, vi } from 'vitest';

// specs/0083 W2b — the wrap-up's two preferences live in Rust now (it runs with the window
// hidden). The first launch after the upgrade pushes the old localStorage values once; every
// later launch pulls Rust's values into the cache.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { syncWrapUpPreferences, WRAP_UP_PREFS_MIGRATED_KEY } from '@/lib/wrap-up-preferences';

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe('syncWrapUpPreferences', () => {
  it('pushes the values the user set, once', async () => {
    localStorage.setItem('isAutoSummary', 'false');
    localStorage.setItem('summaryLanguageDefault', 'fr');
    invoke.mockResolvedValue({ autoSummary: false, summaryLanguageDefault: 'fr' });

    await syncWrapUpPreferences();
    expect(invoke).toHaveBeenCalledWith('api_set_wrap_up_preferences', {
      preferences: { autoSummary: false, summaryLanguageDefault: 'fr' },
    });
    expect(localStorage.getItem(WRAP_UP_PREFS_MIGRATED_KEY)).toBe('true');

    invoke.mockClear();
    invoke.mockResolvedValue({ autoSummary: false, summaryLanguageDefault: 'fr' });
    await syncWrapUpPreferences();
    expect(invoke).toHaveBeenCalledWith('api_get_wrap_up_preferences');
    expect(invoke).not.toHaveBeenCalledWith('api_set_wrap_up_preferences', expect.anything());
  });

  it('sends nothing the user never set (Rust keeps its defaults)', async () => {
    invoke.mockResolvedValue({ autoSummary: true, summaryLanguageDefault: null });
    await syncWrapUpPreferences();
    expect(invoke).toHaveBeenCalledWith('api_set_wrap_up_preferences', { preferences: {} });
  });

  it('after the migration, Rust wins and the cache follows it', async () => {
    localStorage.setItem(WRAP_UP_PREFS_MIGRATED_KEY, 'true');
    localStorage.setItem('isAutoSummary', 'true');
    localStorage.setItem('summaryLanguageDefault', 'de');
    invoke.mockResolvedValue({ autoSummary: false, summaryLanguageDefault: null });

    expect(await syncWrapUpPreferences()).toEqual({ autoSummary: false, summaryLanguageDefault: null });
    expect(localStorage.getItem('isAutoSummary')).toBe('false');
    expect(localStorage.getItem('summaryLanguageDefault')).toBeNull();
  });

  it('a stored language Rust refuses does not block the migration', async () => {
    localStorage.setItem('isAutoSummary', 'false');
    localStorage.setItem('summaryLanguageDefault', 'xx');
    invoke
      .mockRejectedValueOnce('Unsupported summary language. Choose a language from the list.')
      .mockResolvedValueOnce({ autoSummary: false, summaryLanguageDefault: null });

    await syncWrapUpPreferences();
    expect(invoke).toHaveBeenLastCalledWith('api_set_wrap_up_preferences', {
      preferences: { autoSummary: false },
    });
    expect(localStorage.getItem(WRAP_UP_PREFS_MIGRATED_KEY)).toBe('true');
  });
});
