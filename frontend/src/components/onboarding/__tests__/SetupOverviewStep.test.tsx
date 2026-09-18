import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// specs/0061 W1 Task 1 — Setup Overview step gets an inline description per
// step (replacing the dead info-icon tooltip), a size label per model, and a
// free-space warning driven by the new `get_models_disk_check` command.
// Mocks invoke (recommended model + disk check), OnboardingContext, and
// plugin-os the same way other onboarding/component tests do.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));

const goNextMock = vi.fn();
vi.mock('@/contexts/OnboardingContext', () => ({
  useOnboarding: () => ({ goNext: goNextMock }),
}));

import { invoke } from '@tauri-apps/api/core';
import { SetupOverviewStep } from '@/components/onboarding/steps/SetupOverviewStep';

const invokeMock = vi.mocked(invoke);

// The brief's Copy constraint mandates this exact sentence pair (with the two
// byte amounts filled in): "Not enough free space: <free> available, about
// <required> needed. Free some space or download anyway." A bare
// /Not enough free space/ prefix match can't tell this apart from a wrong
// rendering (e.g. "... free, ... needed." with no second sentence), so this
// regex spans both clauses.
const FULL_FREE_SPACE_WARNING =
  /Not enough free space: .+ available, about .+ needed\. Free some space or download anyway\./;

interface DiskCheck {
  free_bytes: number;
  required_bytes: number;
  ok: boolean;
  models_dir: string;
}

function routeInvoke(disk: DiskCheck) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'builtin_ai_get_recommended_model':
        return Promise.resolve('qwen3.5:2b');
      case 'get_models_disk_check':
        return Promise.resolve(disk);
      default:
        return Promise.resolve(undefined);
    }
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  goNextMock.mockClear();
});

describe('SetupOverviewStep', () => {
  it('shows no dead info-icon buttons in the step list', async () => {
    routeInvoke({ free_bytes: 1e9, required_bytes: 2e9, ok: false, models_dir: '/x' });
    render(<SetupOverviewStep />);

    await waitFor(() => expect(screen.getByText(FULL_FREE_SPACE_WARNING)).toBeInTheDocument());

    const stepsList = screen.getByTestId('setup-overview-steps');
    const buttons = stepsList.querySelectorAll('button');
    buttons.forEach((button) => {
      expect(button.querySelector('svg')).toBeNull();
    });
  });

  it('shows the inline summarization description', async () => {
    routeInvoke({ free_bytes: 3e9, required_bytes: 2e9, ok: true, models_dir: '/x' });
    render(<SetupOverviewStep />);

    expect(await screen.findByText(/Prefer OpenAI, Claude, or Ollama/)).toBeInTheDocument();
  });

  it('shows the parakeet and summary model size labels', async () => {
    routeInvoke({ free_bytes: 3e9, required_bytes: 2e9, ok: true, models_dir: '/x' });
    render(<SetupOverviewStep />);

    expect(await screen.findByText(/~670 MB/)).toBeInTheDocument();
    expect(await screen.findByText(/~1.2 GiB/)).toBeInTheDocument();
  });

  it('shows a free-space warning and "Download anyway" when disk check fails', async () => {
    routeInvoke({ free_bytes: 1e9, required_bytes: 2e9, ok: false, models_dir: '/x' });
    render(<SetupOverviewStep />);

    expect(await screen.findByText(FULL_FREE_SPACE_WARNING)).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Download anyway' })).toBeInTheDocument();
  });

  it('shows no warning and "Let\'s Go" when disk check passes', async () => {
    routeInvoke({ free_bytes: 3e9, required_bytes: 2e9, ok: true, models_dir: '/x' });
    render(<SetupOverviewStep />);

    expect(await screen.findByRole('button', { name: /Let's Go/ })).toBeInTheDocument();
    expect(screen.queryByText(/Not enough free space/)).not.toBeInTheDocument();
  });
});
