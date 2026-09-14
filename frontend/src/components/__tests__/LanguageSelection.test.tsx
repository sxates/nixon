import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

// spec 0038 WS7.d — LanguageSelection is now the SINGLE global transcription-language
// control (the old in-recording "Language Settings" modal is gone). Changing it writes
// the one global preference through useConfig().setSelectedLanguage, which persists to
// localStorage and syncs `set_language_preference` to Rust.

const { setSelectedLanguage, transcriptModelConfig } = vi.hoisted(() => ({
  setSelectedLanguage: vi.fn(),
  transcriptModelConfig: { provider: 'localWhisper', model: 'large-v3', apiKey: null as string | null },
}));

vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({
    selectedLanguage: 'auto',
    setSelectedLanguage,
    transcriptModelConfig,
  }),
}));

const { toastMock } = vi.hoisted(() => ({
  toastMock: { success: vi.fn(), error: vi.fn() },
}));
vi.mock('sonner', () => ({ toast: toastMock }));

import { LanguageSelection } from '../LanguageSelection';

describe('LanguageSelection (single global control)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    transcriptModelConfig.provider = 'localWhisper';
  });

  it('writes the global language preference when the user picks a language', () => {
    render(<LanguageSelection />);

    const select = screen.getByRole('combobox');
    // The control is NOT disabled — the old modal disabled it while recording,
    // which is the "can't leave Auto-detect" bug this consolidation fixes.
    expect(select).not.toBeDisabled();

    fireEvent.change(select, { target: { value: 'es' } });

    expect(setSelectedLanguage).toHaveBeenCalledWith('es');
    expect(toastMock.success).toHaveBeenCalled();
  });

  it('limits Whisper to the full language list', () => {
    render(<LanguageSelection />);
    const options = screen.getAllByRole('option');
    // Far more than the two auto options — the full Whisper list is available.
    expect(options.length).toBeGreaterThan(50);
    expect(screen.getByRole('option', { name: 'English (en)' })).toBeInTheDocument();
  });

  it('restricts Parakeet to auto-detect only', () => {
    transcriptModelConfig.provider = 'parakeet';
    render(<LanguageSelection />);
    const options = screen.getAllByRole('option');
    // Parakeet supports only auto / auto-translate.
    expect(options).toHaveLength(2);
  });
});
