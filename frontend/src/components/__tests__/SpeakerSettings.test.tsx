import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

// specs/0067 — these settings moved out of Recordings to the Transcription tab, where the
// engine they describe lives. The guarantees came with them: a change made mid-meeting has
// to say it applies to the NEXT recording (spec 0051 WS3), and the voiceprint copy has to
// state that storing other people's is opt-in biometric data (ADR-0007, specs/0061 W6).

const { useSidebarMock, toastInfo, toastSuccess } = vi.hoisted(() => ({
  useSidebarMock: vi.fn(),
  toastInfo: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { info: toastInfo, success: toastSuccess, error: vi.fn() } }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: useSidebarMock }));
vi.mock('@/components/LanguageSelection', () => ({
  LanguageSelection: () => <div data-testid="language-selection" />,
}));

let provider = 'parakeet';
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ transcriptModelConfig: { provider } }),
}));
vi.mock('@/components/ClearVoiceprintsDialog', () => ({ ClearVoiceprintsDialog: () => null }));

import { invoke } from '@tauri-apps/api/core';
import { SpeakerSettings } from '@/components/SpeakerSettings';

const invokeMock = vi.mocked(invoke);

function switchForLabel(label: string): HTMLElement {
  const labelEl = screen.getByText(label);
  const row = labelEl.closest('.flex.items-center.justify-between') as HTMLElement;
  return within(row).getByRole('switch');
}

async function renderSettings() {
  render(<SpeakerSettings />);
  // Diarization is on in the fixture, so its sub-settings are mounted.
  await waitFor(() =>
    expect(screen.getByText('Label speakers live while recording')).toBeInTheDocument(),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  provider = 'parakeet';
  useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_get_diarization_enabled':
        return Promise.resolve(true);
      case 'api_get_live_diarization_enabled':
        return Promise.resolve(false);
      case 'api_get_voiceprint_settings':
        return Promise.resolve({ storeOthersVoiceprints: false, selfEnrollVoiceprint: true });
      default:
        return Promise.resolve(undefined);
    }
  });
});

describe('SpeakerSettings — start-time-only notice (spec 0051 WS3, moved by 0067)', () => {
  it('says live speaker labels apply to the next recording, mid-meeting', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();

    fireEvent.click(switchForLabel('Label speakers live while recording'));

    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(1));
    expect(toastInfo.mock.calls[0][0]).toMatch(/next recording/i);
    expect(toastSuccess).not.toHaveBeenCalled();
  });

  // specs/0076: switching them OFF mid-meeting stops them in that meeting too.
  it('says switching live labels off stops them for this meeting too', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_get_diarization_enabled':
        case 'api_get_live_diarization_enabled':
          return Promise.resolve(true);
        case 'api_get_voiceprint_settings':
          return Promise.resolve({ storeOthersVoiceprints: false, selfEnrollVoiceprint: true });
        default:
          return Promise.resolve(undefined);
      }
    });
    await renderSettings();
    await waitFor(() =>
      expect(switchForLabel('Label speakers live while recording')).toHaveAttribute('aria-checked', 'true'),
    );

    fireEvent.click(switchForLabel('Label speakers live while recording'));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(1));
    expect(toastSuccess.mock.calls[0][0]).toBe('Live speaker labels are off');
    expect(toastSuccess.mock.calls[0][1].description).toMatch(/this meeting too/);
    expect(toastInfo).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith('api_set_live_diarization_enabled', { enabled: false });
  });

  it('falls back to the plain toast when nothing is recording', async () => {
    await renderSettings();

    fireEvent.click(switchForLabel('Label speakers live while recording'));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Preference saved'));
    expect(toastInfo).not.toHaveBeenCalled();
  });

  it("treats the 'intro-call' placeholder id as no active recording", async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'intro-call' });
    await renderSettings();

    fireEvent.click(switchForLabel('Label speakers live while recording'));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Preference saved'));
    expect(toastInfo).not.toHaveBeenCalled();
  });

  // Diarization itself feeds the POST-meeting pass, so it takes effect immediately and
  // must not claim otherwise — the negative case that keeps the notice honest.
  it('does not add the notice to diarization, even mid-recording', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();

    fireEvent.click(switchForLabel('Speaker diarization'));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Preference saved'));
    expect(toastInfo).not.toHaveBeenCalled();
  });
});

describe('SpeakerSettings — copy (specs/0061 W6, moved by 0067)', () => {
  it('describes live speaker labels with the corrected, honest copy', async () => {
    await renderSettings();
    expect(
      screen.getByText(
        'Shows provisional numbered labels while you record. Names are matched when the recording ends. Uses significant CPU.',
      ),
    ).toBeInTheDocument();
  });

  it('explains that storing other voiceprints is opt-in biometric data', async () => {
    await renderSettings();
    expect(screen.getByText(/biometric data/i)).toBeInTheDocument();
  });
});

// specs/0067 — the transcription-language control is shown only where it does something.
// `parakeet_provider.rs` takes the language, logs that Parakeet does not support it, and
// transcribes anyway; the UI still spent a dropdown and three explanatory blocks saying so.
describe('SpeakerSettings — transcription language', () => {
  it('is absent on the default engine, which ignores it', async () => {
    await renderSettings();
    expect(screen.queryByTestId('language-selection')).toBeNull();
    expect(screen.queryByText('Transcription language')).toBeNull();
  });

  it('appears for Whisper, which honours it', async () => {
    provider = 'localWhisper';
    await renderSettings();
    expect(screen.getByTestId('language-selection')).toBeInTheDocument();
  });

  it('appears for a cloud provider too', async () => {
    provider = 'deepgram';
    await renderSettings();
    expect(screen.getByTestId('language-selection')).toBeInTheDocument();
  });
});
