import { describe, expect, it } from 'vitest';
import {
  AUDIO_FAILED_NOTE,
  SPEAKER_ID_CANCELLED_NOTE,
  SPEAKER_ID_INTERRUPTED_NOTE,
  speakerIdFailureNote,
} from '../useMeetingAudioStatus';

// specs/0084 W1 — the meeting page's failed note follows `speakerIdFailure`.
describe('speakerIdFailureNote', () => {
  it('says a pass that died with the app stopped unexpectedly', () => {
    expect(speakerIdFailureNote('interrupted')).toBe(
      'Speaker identification stopped unexpectedly.',
    );
    expect(speakerIdFailureNote('crashed')).toBe(SPEAKER_ID_INTERRUPTED_NOTE);
  });

  it('says a cancelled pass was cancelled', () => {
    expect(speakerIdFailureNote('cancelled')).toBe(SPEAKER_ID_CANCELLED_NOTE);
    expect(SPEAKER_ID_CANCELLED_NOTE).toBe('Speaker identification was cancelled.');
  });

  it('keeps the existing note for an ordinary error or no code', () => {
    for (const code of ['error', null, undefined, 'something-new']) {
      expect(speakerIdFailureNote(code)).toBe(AUDIO_FAILED_NOTE);
    }
  });
});
