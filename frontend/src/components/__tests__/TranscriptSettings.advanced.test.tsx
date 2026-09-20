import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0067 W2 — the Transcription tab offered two engines and, through Whisper, twelve
// models. Nixon ships Parakeet on its default model and the WER comparison backs it
// (17.6% vs 20.6% on the same corpus, and it is the engine built for the live path), so a
// user running that configuration is shown one row rather than a decision.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/components/WhisperModelManager', () => ({ ModelManager: () => <div data-testid="whisper-models" /> }));
vi.mock('@/components/ParakeetModelManager', () => ({
  ParakeetModelManager: () => <div data-testid="parakeet-models" />,
}));

import { TranscriptSettings } from '@/components/TranscriptSettings';

const DEFAULT_PARAKEET = 'parakeet-tdt-0.6b-v3-int8';

function renderWith(config: { provider: string; model: string }) {
  render(
    <TranscriptSettings
      transcriptModelConfig={{ ...config, apiKey: null } as never}
      setTranscriptModelConfig={vi.fn()}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  invokeMock.mockResolvedValue(undefined);
});

describe('Transcription engine — resolved by default (specs/0067 W2)', () => {
  it('states the engine instead of offering a menu, on the shipped configuration', () => {
    renderWith({ provider: 'parakeet', model: DEFAULT_PARAKEET });

    expect(screen.getByTestId('resolved-value')).toHaveTextContent('Parakeet');
    // Neither the engine picker nor the model list: "Change" reveals them together, so a
    // collapsed row cannot leave the model list showing underneath it.
    expect(screen.queryByTestId('parakeet-models')).toBeNull();
    expect(screen.queryByTestId('whisper-models')).toBeNull();
  });

  it('reveals the engine picker and its model manager together, on Change', () => {
    renderWith({ provider: 'parakeet', model: DEFAULT_PARAKEET });

    fireEvent.click(screen.getByRole('button', { name: /change/i }));

    expect(screen.queryByTestId('resolved-value')).toBeNull();
    expect(screen.getByTestId('parakeet-models')).toBeInTheDocument();
  });

  // Same rule as the Summary tab: a configuration the user chose stays visible and
  // undoable, whether or not they have ever seen the advanced switch.
  it('keeps the controls for someone running Whisper', () => {
    renderWith({ provider: 'localWhisper', model: 'large-v3-turbo' });

    expect(screen.queryByTestId('resolved-value')).toBeNull();
    expect(screen.getByTestId('whisper-models')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /change/i })).toBeNull();
  });

  it('keeps the controls for a hand-picked Parakeet model', () => {
    renderWith({ provider: 'parakeet', model: 'parakeet-tdt-0.6b-v2' });

    expect(screen.queryByTestId('resolved-value')).toBeNull();
    expect(screen.getByTestId('parakeet-models')).toBeInTheDocument();
  });
});
