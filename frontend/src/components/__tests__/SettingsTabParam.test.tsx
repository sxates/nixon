import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';

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
    betaFeatures: { importAndRetranscribe: true },
    toggleBetaFeature: vi.fn(),
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
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=beta'));
    render(<SettingsPage />);
    const tab = await screen.findByRole('tab', { name: /beta/i });
    expect(tab).toHaveAttribute('data-state', 'active');
  });

  it('falls back to the default tab for an unknown ?tab= value', async () => {
    searchParamsMock.mockReturnValue(new URLSearchParams('tab=not-a-real-tab'));
    render(<SettingsPage />);
    const tab = await screen.findByRole('tab', { name: /^general$/i });
    expect(tab).toHaveAttribute('data-state', 'active');
  });
});
