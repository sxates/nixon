import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// specs/0058 Task 5 — the "Download updates automatically" preference row, wired
// to the backend's api_get_updater_settings / api_set_updater_settings commands.

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({
    isLoadingPreferences: false,
    loadPreferences: vi.fn(),
  }),
}));

import { PreferenceSettings } from '@/components/PreferenceSettings';

describe('PreferenceSettings updates (specs/0058)', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_updater_settings') return { auto_update: true };
      return null;
    });
  });

  it('loads the auto-update setting and saves a change', async () => {
    const user = userEvent.setup();
    render(<PreferenceSettings />);

    const toggle = await waitFor(() => screen.getByLabelText('Download updates automatically'));
    await waitFor(() => expect(toggle).toBeChecked());

    await user.click(toggle);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_set_updater_settings', { autoUpdate: false })
    );
  });
});
