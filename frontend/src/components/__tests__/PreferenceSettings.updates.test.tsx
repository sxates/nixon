import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// specs/0058 Task 5 — the "Download updates automatically" preference row, wired
// to the backend's api_get_updater_settings / api_set_updater_settings commands.

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const toastError = vi.fn();
vi.mock('sonner', () => ({ toast: { error: (...args: unknown[]) => toastError(...args) } }));

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
    toastError.mockReset();
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

  it('leaves the early access setting alone when auto-update changes', async () => {
    const user = userEvent.setup();
    render(<PreferenceSettings />);

    const toggle = await waitFor(() => screen.getByLabelText('Download updates automatically'));
    await waitFor(() => expect(toggle).toBeChecked());
    await user.click(toggle);

    await waitFor(() => expect(toggle).not.toBeChecked());
    expect(invoke).not.toHaveBeenCalledWith('api_set_early_access', expect.anything());
  });
});

describe('PreferenceSettings early access (specs/0086)', () => {
  beforeEach(() => {
    invoke.mockReset();
    toastError.mockReset();
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_updater_settings') return { auto_update: true, early_access: false };
      return null;
    });
  });

  it('renders the switch off by default', async () => {
    render(<PreferenceSettings />);
    const toggle = await waitFor(() => screen.getByLabelText('Early access updates'));
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).not.toBeChecked();
  });

  it('is off when the settings predate early access', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_updater_settings') return { auto_update: true };
      return null;
    });
    render(<PreferenceSettings />);
    const toggle = await waitFor(() => screen.getByLabelText('Early access updates'));
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).not.toBeChecked();
  });

  it('saves a change with api_set_early_access and leaves auto-update alone', async () => {
    const user = userEvent.setup();
    render(<PreferenceSettings />);

    const toggle = await waitFor(() => screen.getByLabelText('Early access updates'));
    await waitFor(() => expect(toggle).toBeEnabled());
    await user.click(toggle);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_set_early_access', { enabled: true })
    );
    expect(toggle).toBeChecked();
    expect(invoke).not.toHaveBeenCalledWith('api_set_updater_settings', expect.anything());
    expect(screen.getByLabelText('Download updates automatically')).toBeChecked();
  });

  it('reverts the switch and says so when saving fails', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_updater_settings') return { auto_update: true, early_access: false };
      if (cmd === 'api_set_early_access') throw new Error('disk full');
      return null;
    });
    const user = userEvent.setup();
    render(<PreferenceSettings />);

    const toggle = await waitFor(() => screen.getByLabelText('Early access updates'));
    await waitFor(() => expect(toggle).toBeEnabled());
    await user.click(toggle);

    await waitFor(() => expect(toggle).not.toBeChecked());
    expect(toastError).toHaveBeenCalledWith("Couldn't save the early access setting.");
  });
});
