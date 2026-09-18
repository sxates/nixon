import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0061 W6 — RecordingSettings used to have a SECOND "Summarize automatically when
// a meeting ends" switch that duplicated this one (same ConfigContext state, two
// surfaces). That row is gone from RecordingSettings now; this is the only toggle left,
// and its copy should explain what the setting DOES rather than just point across tabs.

const { toggleIsAutoSummary } = vi.hoisted(() => ({ toggleIsAutoSummary: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn().mockReturnValue(() => {}) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({
    isAutoSummary: false,
    toggleIsAutoSummary,
  }),
}));
vi.mock('@/components/SummaryLanguageSettings', () => ({
  SummaryLanguageSettings: () => null,
}));
vi.mock('@/components/ModelSettingsModal', () => ({
  ModelSettingsModal: () => null,
}));

import { SummaryModelSettings } from '@/components/SummaryModelSettings';

beforeEach(() => {
  vi.clearAllMocks();
});

describe('SummaryModelSettings — single auto-summary toggle (specs/0061 W6)', () => {
  it('renders exactly one "Summarize automatically when a meeting ends" control', () => {
    render(<SummaryModelSettings />);
    expect(
      screen.getAllByText('Summarize automatically when a meeting ends'),
    ).toHaveLength(1);
  });

  it('describes what the setting does instead of pointing across tabs', () => {
    render(<SummaryModelSettings />);
    expect(
      screen.getByText(
        'Generate an AI summary as soon as a recording stops, using your configured summary model.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Also available in Settings/i)).not.toBeInTheDocument();
  });

  it('toggling calls the shared ConfigContext setter', () => {
    render(<SummaryModelSettings />);
    fireEvent.click(screen.getByRole('switch'));
    expect(toggleIsAutoSummary).toHaveBeenCalledWith(true);
  });
});
