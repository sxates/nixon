import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { DeveloperSettings } from '../DeveloperSettings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
const invokeMock = vi.mocked(invoke);

describe('DeveloperSettings', () => {
  // NOTE: a block-body arrow, not `() => invokeMock.mockReset()`. The implicit-return form
  // hands the hook runner `mockReset()`'s return value (the mock instance itself), which
  // trips a Vitest 2.1.9 hook-return-value handling bug: it deterministically misreports the
  // "renders nothing" test's already-caught rejection as an unhandled rejection. Confirmed by
  // bisecting with a minimal reproduction — swapping only this line's return value fixes it,
  // independent of the component's catch style (`.then().catch()`, two-arg `.then()`, and
  // async/await try/catch were all tried and all failed with the implicit-return form).
  beforeEach(() => { invokeMock.mockReset(); });

  it('renders nothing when dev_get_flags is unavailable (release build)', async () => {
    invokeMock.mockRejectedValue(new Error('command dev_get_flags not found'));
    const { container } = render(<DeveloperSettings />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('dev_get_flags'));
    expect(container).toBeEmptyDOMElement();
  });

  it('shows flags and loads demo data', async () => {
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'dev_get_flags') return { fixtures: true, no_audio: false, fake_downloads: false, reset_onboarding: false };
      if (cmd === 'dev_load_fixtures') return { meetings: 5, people: 7, segments: 1100 };
      return undefined;
    });
    render(<DeveloperSettings />);
    expect(await screen.findByText(/fixtures=demo/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /load demo data/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('dev_load_fixtures', { noAudio: false }));
    expect(await screen.findByText(/5 meetings/)).toBeInTheDocument();
  });

  it('resets onboarding', async () => {
    invokeMock.mockImplementation(async (cmd) => (cmd === 'dev_get_flags' ? { fixtures: false, no_audio: false, fake_downloads: false, reset_onboarding: false } : undefined));
    render(<DeveloperSettings />);
    await screen.findByText(/Developer/);
    fireEvent.click(screen.getByRole('button', { name: /reset onboarding/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('dev_reset_onboarding'));
  });
});
