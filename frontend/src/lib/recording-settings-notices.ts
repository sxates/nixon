/**
 * Settings that only take effect on the NEXT recording (spec 0051 WS3).
 *
 * These three preferences are read once, at recording start, and have no effect on a
 * session already under way:
 *   - `live_transcription_enabled`  (Transcribe in real time during recording)
 *   - `low_power_on_battery`        (Low Power Mode on battery)
 *   - `live_diarization_enabled`    (Label speakers live while recording)
 *
 * Changing one mid-recording still saved and still showed "Preference saved", which
 * reads as "this is now in effect" — the 1.14 owner report. The settings stay editable
 * (a deliberate choice over locking them); they just stop implying they applied.
 *
 * Everything else in Settings → Recording is honest mid-recording and is intentionally
 * absent from this map: Speaker diarization, Expected number of speakers, and the
 * voiceprint toggles all still feed the post-meeting pass, and Summarize automatically
 * is read at stop.
 */

export type StartTimeOnlySetting =
  | 'live-transcription'
  | 'low-power-on-battery'
  | 'live-diarization';

export interface SettingNotice {
  title: string;
  description: string;
}

const NOTICES: Record<StartTimeOnlySetting, SettingNotice> = {
  'live-transcription': {
    title: 'Saved — applies to your next recording',
    description:
      'Use the Live/Deferred button on the recording to change this meeting.',
  },
  'low-power-on-battery': {
    title: 'Saved — applies to your next recording',
    description: 'A recording already under way keeps the mode it started in.',
  },
  'live-diarization': {
    title: 'Saved — applies to your next recording',
    description:
      'Live speaker labels are set up when a recording starts. Speakers are still labelled after this meeting ends.',
  },
};

/** The notice to show after saving `setting`, or null when there's nothing to say. */
export function noticeForSetting(
  setting: StartTimeOnlySetting,
  isRecordingActive: boolean,
): SettingNotice | null {
  return isRecordingActive ? NOTICES[setting] : null;
}
