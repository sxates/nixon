import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, fireEvent } from '@testing-library/react';
import { ThemeProvider, useTheme, THEME_STORAGE_KEY } from '@/contexts/ThemeContext';

// specs/0057 decision 1 — light default, follow the OS, manual override persisted.

type MQ = { matches: boolean; listeners: Array<(e: { matches: boolean }) => void> };
let mq: MQ;

function installMatchMedia(prefersDark: boolean) {
  mq = { matches: prefersDark, listeners: [] };
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: query === '(prefers-color-scheme: dark)' ? mq.matches : false,
    media: query,
    addEventListener: (_: string, cb: (e: { matches: boolean }) => void) => mq.listeners.push(cb),
    removeEventListener: (_: string, cb: (e: { matches: boolean }) => void) => {
      mq.listeners = mq.listeners.filter((l) => l !== cb);
    },
  })) as unknown as typeof window.matchMedia;
}

function Probe() {
  const { preference, resolved, setPreference } = useTheme();
  return (
    <div>
      <span data-testid="pref">{preference}</span>
      <span data-testid="resolved">{resolved}</span>
      <button onClick={() => setPreference('dark')}>dark</button>
      <button onClick={() => setPreference('light')}>light</button>
      <button onClick={() => setPreference('system')}>system</button>
    </div>
  );
}

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove('dark');
});
afterEach(() => vi.restoreAllMocks());

describe('ThemeProvider', () => {
  it('defaults to system and resolves light when the OS is light', () => {
    installMatchMedia(false);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('pref').textContent).toBe('system');
    expect(screen.getByTestId('resolved').textContent).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('resolves dark and sets the .dark class when the OS is dark', () => {
    installMatchMedia(true);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('follows OS changes while preference is system', () => {
    installMatchMedia(false);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    act(() => { mq.matches = true; mq.listeners.forEach((l) => l({ matches: true })); });
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('a manual override wins over the OS and persists', () => {
    installMatchMedia(true);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    fireEvent.click(screen.getByText('light'));
    expect(screen.getByTestId('resolved').textContent).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
    // OS flips to dark — ignored while overridden
    act(() => { mq.listeners.forEach((l) => l({ matches: true })); });
    expect(screen.getByTestId('resolved').textContent).toBe('light');
  });

  it('keeps the pre-paint .dark class when the stored preference is dark on an OS-light machine', () => {
    // Fable final review: the provider hydrates with SSR defaults (system/light) and reads
    // storage in a layout effect; the class effect must not run with those defaults, or it
    // would strip the class the pre-paint script set and flash Faceplate over Deck.
    installMatchMedia(false);
    localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    document.documentElement.classList.add('dark');
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('pref').textContent).toBe('dark');
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('reads a persisted preference on mount and tolerates garbage', () => {
    installMatchMedia(false);
    localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    const { unmount } = render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    unmount();
    localStorage.setItem(THEME_STORAGE_KEY, 'neon');
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('pref').textContent).toBe('system');
  });

  // Drift guard: layout.tsx's pre-paint script inlines the storage key as a string
  // literal (it runs before any module loads, so it cannot import the constant).
  // If THEME_STORAGE_KEY is ever renamed without updating that script, the app
  // flashes the wrong theme on every cold start — catch it here.
  it('keeps the pre-paint script in layout.tsx on the same storage key', () => {
    const layout = readFileSync(path.join(__dirname, '..', '..', 'app', 'layout.tsx'), 'utf8');
    expect(layout).toContain(`localStorage.getItem('${THEME_STORAGE_KEY}')`);
  });
});
