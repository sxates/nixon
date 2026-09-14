'use client';

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
} from 'react';

/**
 * specs/0057 decision 1 — Faceplate (light) is the default look; the app follows the
 * macOS appearance unless the user overrides it in Settings → General → Appearance.
 *
 * The resolved theme is applied as the `dark` class on <html> (Tailwind
 * `darkMode: ['class']`, and the `.dark` token block in globals.css). Nothing else in
 * the tree should touch that class.
 *
 * Hydration: the static export renders with the SSR defaults (`system` / OS light), and
 * the FIRST client render must match it — React 18 never repairs attribute mismatches,
 * so initialising state from localStorage here left `aria-checked` on the Appearance
 * radios and Sonner's `data-sonner-theme` frozen at the server value until a reload.
 * The real preference is read in a layout effect (before paint; the pre-paint script in
 * app/layout.tsx already has the `.dark` class right, so there is no flash), and the
 * class effect waits for that read so it never toggles `.dark` off with the defaults.
 *
 * Note: the Tauri window config must NOT pin `"theme": "Light"` — that forces the
 * WebView's prefers-color-scheme to light and `system` would never resolve dark.
 */
export type ThemePreference = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'nixon.theme';
const DARK_QUERY = '(prefers-color-scheme: dark)';

interface ThemeContextValue {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (p: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function readStoredPreference(): ThemePreference {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    return v === 'light' || v === 'dark' || v === 'system' ? v : 'system';
  } catch {
    return 'system';
  }
}

function osPrefersDark(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(DARK_QUERY).matches
    : false;
}

// useLayoutEffect warns during SSR; the export prerender has no layout to measure anyway.
const useIsomorphicLayoutEffect = typeof window !== 'undefined' ? useLayoutEffect : useEffect;

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>('system');
  const [systemDark, setSystemDark] = useState<boolean>(false);
  const [hydrated, setHydrated] = useState(false);

  // Read the persisted preference + OS appearance before first paint (see header).
  useIsomorphicLayoutEffect(() => {
    setPreferenceState(readStoredPreference());
    setSystemDark(osPrefersDark());
    setHydrated(true);
  }, []);

  // Track the OS appearance. Only matters while preference === 'system', but keeping the
  // listener always-on means switching back to System is instant and correct.
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(DARK_QUERY);
    const onChange = (e: { matches: boolean }) => setSystemDark(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  const resolved: ResolvedTheme =
    preference === 'system' ? (systemDark ? 'dark' : 'light') : preference;

  useEffect(() => {
    if (!hydrated) return; // the pre-paint script owns the class until we've read storage
    document.documentElement.classList.toggle('dark', resolved === 'dark');
  }, [hydrated, resolved]);

  const setPreference = useCallback((p: ThemePreference) => {
    setPreferenceState(p);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, p);
    } catch {
      /* private mode / quota — the in-memory preference still applies this session */
    }
  }, []);

  const value = useMemo(() => ({ preference, resolved, setPreference }), [preference, resolved, setPreference]);
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within a ThemeProvider');
  return ctx;
}
