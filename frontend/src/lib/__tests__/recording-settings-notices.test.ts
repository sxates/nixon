import { describe, it, expect } from 'vitest';
import {
  noticeForSetting,
  type StartTimeOnlySetting,
} from '@/lib/recording-settings-notices';

// spec 0051 WS3 — three Recording settings are read ONCE at recording start and do
// nothing to a session already under way. The owner changed one mid-meeting, saw
// "Preference saved", and reasonably assumed it had taken effect.

const ALL: StartTimeOnlySetting[] = [
  'live-transcription',
  'low-power-on-battery',
  'live-diarization',
];

describe('noticeForSetting', () => {
  it('says nothing when no recording is active', () => {
    for (const setting of ALL) {
      expect(noticeForSetting(setting, false)).toBeNull();
    }
  });

  it('returns a notice for every start-time-only setting during a recording', () => {
    for (const setting of ALL) {
      const notice = noticeForSetting(setting, true);
      expect(notice, setting).not.toBeNull();
      expect(notice!.title).toMatch(/next recording/i);
      expect(notice!.description.length).toBeGreaterThan(0);
    }
  });

  it('points live-transcription at the control that DOES affect this meeting', () => {
    const notice = noticeForSetting('live-transcription', true);
    expect(notice!.description).toMatch(/Live\/Deferred/i);
  });

  it('reassures that live-diarization only affects labels DURING the meeting', () => {
    const notice = noticeForSetting('live-diarization', true);
    expect(notice!.description).toMatch(/after this meeting ends/i);
  });
});
