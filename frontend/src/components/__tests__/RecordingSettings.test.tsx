import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

// spec 0051 WS3 — wiring coverage for RecordingSettings.tsx. The lib-level test
// (recording-settings-notices.test.ts) proves noticeForSetting's own logic; this file
// proves each handler actually calls it with the RIGHT setting key, that
// `isRecordingActive` is derived correctly from `activeRecordingMeetingId`, and that
// settings NOT in the start-time-only map (e.g. Speaker diarization) keep the plain
// "Preference saved" toast with no notice — the wrong-direction mistake the brief
// explicitly warns against.

const { useSidebarMock } = vi.hoisted(() => ({ useSidebarMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('@tauri-apps/plugin-store', () => ({
  Store: {
    load: vi.fn().mockResolvedValue({
      get: vi.fn().mockResolvedValue(true),
      set: vi.fn().mockResolvedValue(undefined),
      save: vi.fn().mockResolvedValue(undefined),
    }),
  },
}));
vi.mock('sonner', () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({
    isAutoSummary: false,
    toggleIsAutoSummary: vi.fn(),
    selectedLanguage: 'auto',
    setSelectedLanguage: vi.fn(),
    transcriptModelConfig: { provider: 'whisper' },
  }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: useSidebarMock,
}));

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { RecordingSettings } from '@/components/RecordingSettings';

const invokeMock = vi.mocked(invoke);
const toastSuccess = vi.mocked(toast.success);
const toastInfo = vi.mocked(toast.info);

function mockInvokeResponses() {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'get_recording_preferences':
        return Promise.resolve({
          save_folder: '',
          auto_save: true,
          preferred_mic_device: null,
          preferred_system_device: null,
          retention_days: null,
          live_transcription_enabled: true,
          low_power_on_battery: true,
        });
      case 'api_get_zoom_auto_detect':
        return Promise.resolve(true);
      // Diarization on so the "Label speakers live" sub-toggle renders.
      case 'api_get_diarization_enabled':
        return Promise.resolve(true);
      case 'api_get_live_diarization_enabled':
        return Promise.resolve(false);
      case 'api_get_expected_speaker_count':
        return Promise.resolve(null);
      case 'api_get_voiceprint_settings':
        return Promise.resolve({ storeOthersVoiceprints: false, selfEnrollVoiceprint: true });
      case 'api_get_zoom_mute_gate':
        return Promise.resolve(false);
      case 'get_audio_devices':
        return Promise.resolve([]);
      case 'get_audio_backend_info':
        return Promise.resolve([]);
      case 'get_current_audio_backend':
        return Promise.resolve('coreaudio');
      default:
        return Promise.resolve(undefined);
    }
  });
}

/** Finds the Switch inside the row whose visible label is `label`. */
function switchForLabel(label: string): HTMLElement {
  const labelEl = screen.getByText(label);
  const row = labelEl.closest('.flex.items-center.justify-between') as HTMLElement;
  return within(row).getByRole('switch');
}

async function renderSettings() {
  render(<RecordingSettings />);
  // Loading skeleton clears once get_recording_preferences resolves.
  await screen.findByText('Transcribe in real time during recording');
  // Let the other mount-time useEffects (diarization, live-diarization, etc.) settle.
  await waitFor(() => expect(screen.getByText('Label speakers live while recording')).toBeInTheDocument());
}

beforeEach(() => {
  vi.clearAllMocks();
  mockInvokeResponses();
});

describe('RecordingSettings — start-time-only setting notices (spec 0051 WS3)', () => {
  it('shows the "next recording" notice for all three start-time-only settings while a recording is active', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();

    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(1));
    expect(toastInfo.mock.calls[0][0]).toMatch(/next recording/i);
    expect(toastInfo.mock.calls[0][1]?.description).toMatch(/Live\/Deferred/i);

    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(2));
    expect(toastInfo.mock.calls[1][1]?.description).toMatch(/already under way/i);

    fireEvent.click(switchForLabel('Label speakers live while recording'));
    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(3));
    expect(toastInfo.mock.calls[2][1]?.description).toMatch(/after this meeting ends/i);

    // None of these should have fallen through to the plain toast.
    expect(toastSuccess).not.toHaveBeenCalled();
  });

  it('keeps the plain "Preference saved" toast (no notice) when no recording is active', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    await renderSettings();

    // live-transcription and low-power keep their own state-dependent description —
    // that's the regression coverage below. This test just confirms no toast.info fires
    // and live-diarization (which never had a description) stays bare.
    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(1));

    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(2));

    fireEvent.click(switchForLabel('Label speakers live while recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(3));
    expect(toastSuccess.mock.calls[2]).toEqual(['Preference saved']);

    expect(toastInfo).not.toHaveBeenCalled();
  });

  // Regression coverage: task-6 originally replaced these two handlers' toast calls
  // wholesale with toastSaved(setting), which silently dropped their state-dependent
  // description in the (common) no-active-recording case. toastSaved now takes a
  // fallbackDescription so the notice only wins while a recording is active.
  it('preserves live-transcription\'s state-dependent description when no recording is active (both branches)', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    await renderSettings();

    // Mock starts live_transcription_enabled: true, so the first click turns it OFF.
    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(1));
    expect(toastSuccess).toHaveBeenNthCalledWith(1, 'Preference saved', {
      description:
        'Recording only — meetings are transcribed later (automatically before a summary, or with “Transcribe now”).',
    });

    // Click again to turn it back ON.
    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(2));
    expect(toastSuccess).toHaveBeenNthCalledWith(2, 'Preference saved', {
      description: 'Meetings will be transcribed live while you record.',
    });

    expect(toastInfo).not.toHaveBeenCalled();
  });

  it('preserves low-power-on-battery\'s state-dependent description when no recording is active (both branches)', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    await renderSettings();

    // Mock starts low_power_on_battery: true, so the first click turns it OFF.
    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(1));
    expect(toastSuccess).toHaveBeenNthCalledWith(1, 'Preference saved', {
      description: 'Meetings are transcribed live even on battery.',
    });

    // Click again to turn it back ON.
    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(2));
    expect(toastSuccess).toHaveBeenNthCalledWith(2, 'Preference saved', {
      description:
        "On battery power, meetings are recorded only — transcription and summaries wait until you're plugged in.",
    });

    expect(toastInfo).not.toHaveBeenCalled();
  });

  it("treats the 'intro-call' placeholder id as no active recording", async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'intro-call' });
    await renderSettings();

    fireEvent.click(switchForLabel('Label speakers live while recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Preference saved'));
    expect(toastInfo).not.toHaveBeenCalled();
  });

  it('does NOT add a notice to Speaker diarization even during an active recording (negative case)', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();

    // Speaker diarization feeds the post-meeting pass — it must stay on the plain
    // toast even mid-recording, unlike the three start-time-only settings above.
    fireEvent.click(switchForLabel('Speaker diarization'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Preference saved'));
    expect(toastInfo).not.toHaveBeenCalled();
  });
});

describe('RecordingSettings — settings hygiene (specs/0061 W6)', () => {
  beforeEach(() => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
  });

  it('has no File format row (dead: format was never actually configurable)', async () => {
    await renderSettings();
    expect(screen.queryByText('File format')).not.toBeInTheDocument();
  });

  it('has no second auto-summary switch, and points to Summary instead', async () => {
    await renderSettings();
    // Only one control with this aria-label may exist app-wide; RecordingSettings no
    // longer renders one of its own.
    expect(screen.queryByLabelText('Summarize automatically when a meeting ends')).not.toBeInTheDocument();
    expect(
      screen.getByText('Automatic summaries are configured under Summary.'),
    ).toBeInTheDocument();
  });

  it('describes live speaker labels with the corrected, honest copy', async () => {
    await renderSettings();
    expect(
      screen.getByText(
        'Shows provisional numbered labels while you record. Names are matched when the recording ends.',
      ),
    ).toBeInTheDocument();
  });

  it('explains that storing other voiceprints is opt-in biometric data', async () => {
    await renderSettings();
    expect(
      screen.getByText(
        "Off by default. Storing other people's voiceprints is opt-in because it is biometric data. When on, Nixon remembers other people's voices to suggest names automatically in future meetings. When off, names still suggest within a single meeting, but no cross-meeting voice memory is kept for others. Voiceprints stay on this Mac either way.",
      ),
    ).toBeInTheDocument();
  });

  it('shows both "Open folder" and "Change…" for the save location', async () => {
    await renderSettings();
    expect(screen.getByRole('button', { name: /Open folder/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeInTheDocument();
  });

  it('Change… picks a folder via select_recording_folder and persists it', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'get_recording_preferences':
          return Promise.resolve({
            save_folder: '/tmp/original-folder',
            auto_save: true,
            preferred_mic_device: null,
            preferred_system_device: null,
            retention_days: null,
            live_transcription_enabled: true,
            low_power_on_battery: true,
          });
        case 'api_get_zoom_auto_detect':
          return Promise.resolve(true);
        case 'api_get_diarization_enabled':
          return Promise.resolve(false);
        case 'api_get_live_diarization_enabled':
          return Promise.resolve(false);
        case 'api_get_expected_speaker_count':
          return Promise.resolve(null);
        case 'api_get_voiceprint_settings':
          return Promise.resolve({ storeOthersVoiceprints: false, selfEnrollVoiceprint: true });
        case 'api_get_zoom_mute_gate':
          return Promise.resolve(false);
        case 'get_audio_devices':
          return Promise.resolve([]);
        case 'get_audio_backend_info':
          return Promise.resolve([]);
        case 'get_current_audio_backend':
          return Promise.resolve('coreaudio');
        case 'select_recording_folder':
          return Promise.resolve('/tmp/chosen-folder');
        case 'set_recording_preferences':
          return Promise.resolve(undefined);
        default:
          return Promise.resolve(undefined);
      }
    });

    render(<RecordingSettings />);
    await screen.findByText('Transcribe in real time during recording');

    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('select_recording_folder'));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_recording_preferences', {
        preferences: expect.objectContaining({ save_folder: '/tmp/chosen-folder' }),
      }),
    );
  });

  it('Change… does nothing when the picker is cancelled (no path returned)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'get_recording_preferences':
          return Promise.resolve({
            save_folder: '/tmp/original-folder',
            auto_save: true,
            preferred_mic_device: null,
            preferred_system_device: null,
            retention_days: null,
            live_transcription_enabled: true,
            low_power_on_battery: true,
          });
        case 'api_get_zoom_auto_detect':
          return Promise.resolve(true);
        case 'api_get_diarization_enabled':
          return Promise.resolve(false);
        case 'api_get_live_diarization_enabled':
          return Promise.resolve(false);
        case 'api_get_expected_speaker_count':
          return Promise.resolve(null);
        case 'api_get_voiceprint_settings':
          return Promise.resolve({ storeOthersVoiceprints: false, selfEnrollVoiceprint: true });
        case 'api_get_zoom_mute_gate':
          return Promise.resolve(false);
        case 'get_audio_devices':
          return Promise.resolve([]);
        case 'get_audio_backend_info':
          return Promise.resolve([]);
        case 'get_current_audio_backend':
          return Promise.resolve('coreaudio');
        case 'select_recording_folder':
          return Promise.resolve(null);
        default:
          return Promise.resolve(undefined);
      }
    });

    render(<RecordingSettings />);
    await screen.findByText('Transcribe in real time during recording');

    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('select_recording_folder'));
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'set_recording_preferences')).toBe(false);
  });
});
