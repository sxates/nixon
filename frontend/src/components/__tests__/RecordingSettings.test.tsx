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
  // Let the remaining mount-time effects settle. This used to anchor on "Label speakers
  // live while recording", which specs/0067 moved to the Transcription tab — the last
  // section on this tab is Audio storage, so wait for that instead.
  await waitFor(() => expect(screen.getByText('Delete audio recordings')).toBeInTheDocument());
}

beforeEach(() => {
  vi.clearAllMocks();
  mockInvokeResponses();
});

describe('RecordingSettings — start-time-only setting notices (spec 0051 WS3)', () => {
  // specs/0067 moved "Label speakers live while recording" to the Transcription tab, so
  // its half of this guarantee now lives in SpeakerSettings.test.tsx. The rule is the
  // same in both places: a change made mid-meeting says it applies to the NEXT recording.
  it('shows the "next recording" notice for the start-time-only settings while a recording is active', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();

    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(1));
    expect(toastInfo.mock.calls[0][0]).toMatch(/next recording/i);
    expect(toastInfo.mock.calls[0][1]?.description).toMatch(/Live\/Deferred/i);

    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(2));
    expect(toastInfo.mock.calls[1][1]?.description).toMatch(/already under way/i);

    // None of these should have fallen through to the plain toast.
    expect(toastSuccess).not.toHaveBeenCalled();
  });

  it('keeps the plain "Preference saved" toast (no notice) when no recording is active', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    await renderSettings();

    // live-transcription and low-power keep their own state-dependent description —
    // that's the regression coverage below. This test just confirms no toast.info fires.
    fireEvent.click(switchForLabel('Transcribe in real time during recording'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(1));

    fireEvent.click(switchForLabel('Low Power Mode on battery'));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledTimes(2));

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
});

describe('RecordingSettings — settings hygiene (specs/0061 W6)', () => {
  beforeEach(() => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
  });

  it('has no File format row (dead: format was never actually configurable)', async () => {
    await renderSettings();
    expect(screen.queryByText('File format')).not.toBeInTheDocument();
  });

  // specs/0061 W6 removed a duplicate auto-summary switch from this tab and left a note
  // pointing at Summary. specs/0067 removed the note too: a settings tab does not need to
  // narrate what the other tabs contain. The guarantee that matters is unchanged — only
  // one control for that setting exists app-wide, and it is not here.
  it('has no second auto-summary switch, and no signpost to one', async () => {
    await renderSettings();
    expect(screen.queryByLabelText('Summarize automatically when a meeting ends')).not.toBeInTheDocument();
    expect(screen.queryByText(/configured under Summary/i)).not.toBeInTheDocument();
  });

  // specs/0066 W1 — the expected-speaker override is gone. The audio-derived seed
  // (specs/0050) sizes a meeting now, and a control whose stored value outranked every
  // derived bound is exactly what should not be left lying around.
  it('offers no expected-speaker-count override', async () => {
    await renderSettings();
    expect(screen.queryByLabelText('Expected number of speakers')).not.toBeInTheDocument();
    expect(screen.queryByText('Expected number of speakers')).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_expected_speaker_count', expect.anything());
  });

  it('shows both "Open folder" and "Change…" for the save location', async () => {
    await renderSettings();
    expect(screen.getByRole('button', { name: /Open folder/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeInTheDocument();
  });

  it('Change… picks a folder via select_recording_folder and changes it through the mover', async () => {
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
        case 'select_recording_folder':
          return Promise.resolve('/tmp/chosen-folder');
        // specs/0073: nothing to move, so no dialog — the folder just changes.
        case 'api_plan_recordings_move':
          return Promise.resolve({ target: '/tmp/chosen-folder', meetings: 0, bytes: 0, enoughSpace: true });
        default:
          return Promise.resolve(undefined);
      }
    });

    render(<RecordingSettings />);
    await screen.findByText('Transcribe in real time during recording');

    fireEvent.click(screen.getByRole('button', { name: 'Change…' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('select_recording_folder'));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_change_recordings_folder', {
        target: '/tmp/chosen-folder',
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
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'api_change_recordings_folder')).toBe(false);
  });

  // specs/0073 — the folder can't change under a running recording.
  it('disables Change… while a recording is active', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: 'meeting-123' });
    await renderSettings();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled();
  });

  it('enables Change… when nothing is recording', async () => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    await renderSettings();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeEnabled();
  });
});

describe('RecordingSettings — audio storage copy (specs/0072 W3)', () => {
  beforeEach(() => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
    const base = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation((cmd: string, args?: unknown) =>
      cmd === 'get_recording_preferences'
        ? Promise.resolve({
            save_folder: '/tmp/rec',
            auto_save: false,
            audio_retention: { mode: 'after_processing' },
            preferred_mic_device: null,
            preferred_system_device: null,
            retention_days: 30,
            live_transcription_enabled: true,
            low_power_on_battery: true,
          })
        : base(cmd, args as never),
    );
  });

  it('shows the save location under Once processed, because capture always saves', async () => {
    await renderSettings();
    expect(screen.getByRole('button', { name: 'Change…' })).toBeInTheDocument();
  });

  it('says audio goes only after processing, never at stop, and explains voiceprints', async () => {
    await renderSettings();
    expect(
      screen.getByText(/kept until Nixon has transcribed the meeting and identified the speakers/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/as soon as a recording stops/i)).toBeNull();
    expect(screen.getByText(/numeric voice signatures that contain no audio/)).toBeInTheDocument();
    expect(screen.getByText(/still rename and reassign its speakers/)).toBeInTheDocument();
  });
});

describe('RecordingSettings — meeting detection (specs/0074 W5)', () => {
  beforeEach(() => {
    useSidebarMock.mockReturnValue({ activeRecordingMeetingId: null });
  });

  // Named for what it does rather than one app; the stored key keeps its old name.
  it('is called "Detect meetings" and still saves to the same key', async () => {
    await renderSettings();
    expect(screen.queryByText(/Auto-detect Zoom/)).not.toBeInTheDocument();
    // specs/0074 W6: Teams and Google Meet are detected too, and the copy says so.
    expect(
      screen.getByText('When a Zoom, Teams or Google Meet call starts, offer to record it.'),
    ).toBeInTheDocument();
    fireEvent.click(switchForLabel('Detect meetings'));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_zoom_auto_detect', { enabled: false }),
    );
  });
});
