import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// specs/0067 W1 — the Summary tab stops asking which model to use. Nixon already decides
// (`database/commands.rs` writes `recommend_summary_model(ram)` as the default on first
// run), so the menu was asking a question the app had answered, in names — "Qwen 3.5 2B",
// "gemma3:4b" — that mean nothing to anyone who is not already an AI hobbyist.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn().mockReturnValue(() => {}) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/components/SummaryLanguageSettings', () => ({ SummaryLanguageSettings: () => null }));
vi.mock('@/components/ModelSettingsModal', () => ({
  ModelSettingsModal: () => <div data-testid="model-picker">picker</div>,
}));

let showAdvanced = false;
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ isAutoSummary: true, toggleIsAutoSummary: vi.fn(), showAdvanced }),
}));

import { SummaryModelSettings } from '@/components/SummaryModelSettings';

/** Backend responses for one scenario. */
function arrange({ provider, model }: { provider: string; model: string }) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_get_model_config':
        return Promise.resolve({ provider, model, whisperModel: 'large-v3', apiKeyConfigured: false });
      case 'builtin_ai_get_recommended_model':
        return Promise.resolve('qwen3.5:4b');
      case 'builtin_ai_list_models':
        return Promise.resolve([
          { name: 'qwen3.5:4b', display_name: 'Qwen 3.5 4B (High Quality)' },
          { name: 'qwen3.5:2b', display_name: 'Qwen 3.5 2B (Balanced)' },
        ]);
      default:
        return Promise.resolve(undefined);
    }
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  showAdvanced = false;
});

describe('Summary model — resolved by default (specs/0067 W1)', () => {
  it('shows what is in use instead of a picker, when the recommendation is in use', async () => {
    arrange({ provider: 'builtin-ai', model: 'qwen3.5:4b' });
    render(<SummaryModelSettings />);

    // The row renders before its label resolves (it shows "…" until
    // `builtin_ai_list_models` answers), so wait for the content, not the element —
    // `findByTestId` alone passes the moment the placeholder appears and loses the race
    // under load.
    await waitFor(() =>
      expect(screen.getByTestId('resolved-summary-model')).toHaveTextContent('Qwen 3.5 4B'),
    );
    expect(screen.queryByTestId('model-picker')).toBeNull();
  });

  it('drops the marketing parenthetical from the model name', async () => {
    arrange({ provider: 'builtin-ai', model: 'qwen3.5:4b' });
    render(<SummaryModelSettings />);

    await waitFor(() =>
      expect(screen.getByTestId('resolved-summary-model')).toHaveTextContent('Qwen 3.5 4B'),
    );
    expect(screen.getByTestId('resolved-summary-model').textContent).not.toMatch(/High Quality/);
  });

  it('brings the picker back when advanced options are on', async () => {
    showAdvanced = true;
    arrange({ provider: 'builtin-ai', model: 'qwen3.5:4b' });
    render(<SummaryModelSettings />);

    expect(await screen.findByTestId('model-picker')).toBeInTheDocument();
    expect(screen.queryByTestId('resolved-summary-model')).toBeNull();
  });

  // The rule that keeps this from becoming a support question: someone running a model
  // they chose — or a cloud provider — must keep seeing the control, switch or no switch.
  it('keeps the picker for a model the user chose themselves', async () => {
    arrange({ provider: 'builtin-ai', model: 'qwen3.5:2b' }); // recommendation is 4b
    render(<SummaryModelSettings />);

    expect(await screen.findByTestId('model-picker')).toBeInTheDocument();
  });

  it('keeps the picker for a cloud provider', async () => {
    arrange({ provider: 'anthropic', model: 'claude-3-5-sonnet' });
    render(<SummaryModelSettings />);

    expect(await screen.findByTestId('model-picker')).toBeInTheDocument();
  });

  it('falls back to the picker when the recommendation cannot be read', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_model_config') {
        return Promise.resolve({ provider: 'builtin-ai', model: 'qwen3.5:4b' });
      }
      if (cmd === 'builtin_ai_get_recommended_model') return Promise.reject('no');
      return Promise.resolve(undefined);
    });
    render(<SummaryModelSettings />);

    // Unknown recommendation means we cannot claim the default is in use, so show the
    // controls rather than hide a setting we cannot vouch for.
    await waitFor(() => expect(screen.getByTestId('model-picker')).toBeInTheDocument());
  });
});
