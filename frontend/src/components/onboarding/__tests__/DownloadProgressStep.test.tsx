import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// specs/0061 W1 Task 2 — the first external user found this screen fired a
// top-right "Downloads will continue in the background" toast on Continue,
// duplicating the in-page download progress cards already shown here. That
// toast is gone; Continue should just advance (goNext on macOS) with no
// toast.info call, regardless of whether the summary model is still
// downloading in the background.

vi.mock('sonner', () => ({
  toast: { info: vi.fn(), success: vi.fn(), error: vi.fn() },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));

const goNextMock = vi.fn();
const completeOnboardingMock = vi.fn();
const startBackgroundDownloadsMock = vi.fn().mockResolvedValue(undefined);

vi.mock('@/contexts/OnboardingContext', () => ({
  useOnboarding: () => ({
    goNext: goNextMock,
    selectedSummaryModel: 'qwen3.5:2b',
    recommendedSummaryModel: 'qwen3.5:2b',
    parakeetDownloaded: true,
    setParakeetDownloaded: vi.fn(),
    summaryModelDownloaded: false,
    setSummaryModelDownloaded: vi.fn(),
    startBackgroundDownloads: startBackgroundDownloadsMock,
    completeOnboarding: completeOnboardingMock,
  }),
}));

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { DownloadProgressStep } from '@/components/onboarding/steps/DownloadProgressStep';

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'parakeet_init':
        return Promise.resolve();
      case 'parakeet_has_available_models':
        return Promise.resolve(true);
      case 'builtin_ai_get_recommended_model':
        return Promise.resolve('qwen3.5:2b');
      default:
        return Promise.resolve(undefined);
    }
  });
  vi.mocked(toast.info).mockClear();
  goNextMock.mockClear();
  startBackgroundDownloadsMock.mockClear();
});

describe('DownloadProgressStep', () => {
  it('does not show the "continue in background" toast and advances via goNext on Continue', async () => {
    render(<DownloadProgressStep />);

    const continueButton = await screen.findByRole('button', { name: 'Continue' });
    fireEvent.click(continueButton);

    await waitFor(() => expect(goNextMock).toHaveBeenCalled());

    expect(toast.info).not.toHaveBeenCalled();
  });
});
