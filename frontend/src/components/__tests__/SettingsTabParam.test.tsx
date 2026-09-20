import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// specs/0060 — the screenshot pipeline reaches a specific settings tab via
// `/settings?tab=<value>`. Everything below is scaffolding so SettingsPage (which pulls
// in every settings sub-component) can render outside the app's real provider tree.

const { searchParamsMock } = vi.hoisted(() => ({ searchParamsMock: vi.fn() }));
vi.mock('next/navigation', () => ({ useSearchParams: searchParamsMock, useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({
    transcriptModelConfig: { provider: 'localWhisper', model: 'large-v3', apiKey: null },
    setTranscriptModelConfig: vi.fn(),
    notificationSettings: null,
    isLoadingPreferences: false,
    loadPreferences: vi.fn(),
    updateNotificationSettings: vi.fn(),
    isAutoSummary: false,
    toggleIsAutoSummary: vi.fn(),
  }),
}));
vi.mock('@/contexts/ThemeContext', () => ({
  useTheme: () => ({ preference: 'system', resolved: 'light', setPreference: vi.fn() }),
}));
vi.mock('@/contexts/PermissionsModalContext', () => ({
  usePermissionsModal: () => ({ openPermissionsModal: vi.fn() }),
}));

import SettingsPage from '@/app/settings/page';

describe('settings ?tab= (specs/0060)', () => {
  it('opens the requested tab', async () => {
    // specs/0067 folded Templates into Summary — a template is how a summary is shaped.
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=calendar'));
    render(<SettingsPage />);
    const tab = await screen.findByRole('tab', { name: /calendar/i });
    expect(tab).toHaveAttribute('data-state', 'active');
  });

  // specs/0066 W3 — Import Audio was the only beta feature, so the Beta tab went with it.
  // The debug-only Developer section it used to host is its own tab now, and `dev_get_flags`
  // (a debug-only command) decides whether that tab exists at all. The mock above resolves
  // every invoke, so this stands in for a debug build; a release build's rejection leaves
  // the tab out entirely.
  it('shows a Developer tab only where the debug command answers', async () => {
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=general'));
    render(<SettingsPage />);
    expect(await screen.findByRole('tab', { name: /developer/i })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: /^beta$/i })).not.toBeInTheDocument();
  });

  // specs/0066 — the Developer tab exists only in debug builds, and a real-window README
  // capture runs against exactly such a build. `data-dev-only` is what globals.css hides
  // when a screenshot driver sets `data-shot="1"`, the same mechanism as the DEV badge, so
  // the published images show the shipped app's tab strip.
  it('marks the Developer tab as dev-only chrome so screenshots hide it', async () => {
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=general'));
    render(<SettingsPage />);
    const dev = await screen.findByRole('tab', { name: /developer/i });
    expect(dev).toHaveAttribute('data-dev-only');
    // The shipped tabs must NOT carry it, or a capture would lose the whole strip.
    expect(screen.getByRole('tab', { name: /^general$/i })).not.toHaveAttribute('data-dev-only');
  });

  // The attribute above is inert without the rule that acts on it, and the two live in
  // different files — so pin the pair, the way mock-contract.test.ts pins the screenshot
  // mock against the Rust command registry.
  it('has a stylesheet rule that actually hides dev-only chrome in shot mode', () => {
    const css = readFileSync(join(process.cwd(), 'src/app/globals.css'), 'utf8');
    expect(css).toMatch(/html\[data-shot="1"\]\s*\[data-dev-only\]/);
  });

  it('falls back to the default tab for an unknown ?tab= value', async () => {
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=not-a-real-tab'));
    render(<SettingsPage />);
    const tab = await screen.findByRole('tab', { name: /^general$/i });
    expect(tab).toHaveAttribute('data-state', 'active');
  });
});
