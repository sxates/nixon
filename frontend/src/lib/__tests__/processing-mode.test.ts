import { describe, expect, it } from 'vitest';
import {
  effectiveLiveTranscription,
  isUnprocessedRecording,
  modeChipDisplay,
  shouldAutoDiarizeAtStop,
  stopAction,
  transcriptEmptyStateVariant,
} from '../processing-mode';

describe('stopAction', () => {
  // Signature: stopAction(startedDeferred, everDeferred, overrideMode).
  it('processes immediately when a deferred-started session was overridden live', () => {
    expect(stopAction(true, true, 'live')).toBe('process-now');
  });
  it('marks a still-deferred session for backlog', () => {
    expect(stopAction(true, true, null)).toBe('mark-defer');
  });
  it('marks a live session the user deferred mid-meeting', () => {
    // Live-started, deferred mid-way (chip sets mode='defer') → ever-deferred.
    expect(stopAction(false, true, 'defer')).toBe('mark-defer');
  });
  it('does nothing for a normal live meeting that was never deferred', () => {
    expect(stopAction(false, false, null)).toBe('none');
  });

  // The square of the matrix Finding 1 was about: a session that started LIVE,
  // was flipped to defer mid-recording, then flipped back to live. It has
  // startedDeferred=false but everDeferred=true — its deferred span is a gap and
  // the re-attached VAD restarts timestamps near 0, so it MUST get a repair pass.
  it('repairs a live→defer→live session via ever-deferred', () => {
    expect(stopAction(false, true, 'live')).toBe('process-now');
  });
  // Live-started, deferred mid-way and NOT overridden back to live: its tail is
  // missing, so mark-defer even if no explicit override was recorded.
  it('marks a live→defer (never re-lived) session for backlog even with no override', () => {
    expect(stopAction(false, true, null)).toBe('mark-defer');
  });
});

describe('shouldAutoDiarizeAtStop', () => {
  it('runs stop-time auto-diarization only for a normal live meeting', () => {
    expect(shouldAutoDiarizeAtStop('none')).toBe(true);
  });
  it('skips it for a process-now meeting (the full pass already diarizes)', () => {
    expect(shouldAutoDiarizeAtStop('process-now')).toBe(false);
  });
  it('skips it for a mark-defer meeting — NOTHING runs until manual/AC processing', () => {
    // The 1.10 feedback bug: on battery the meeting was marked deferred but
    // auto-diarization still ran immediately at stop. Deferred meetings get
    // their diarization from the backlog pipeline instead.
    expect(shouldAutoDiarizeAtStop('mark-defer')).toBe(false);
  });
});

describe('effectiveLiveTranscription', () => {
  it('prefers the live event value when present', () => {
    expect(effectiveLiveTranscription(true, false)).toBe(true);
    expect(effectiveLiveTranscription(false, true)).toBe(false);
  });
  it('falls back to the stored preference before any event arrives', () => {
    expect(effectiveLiveTranscription(null, true)).toBe(true);
    expect(effectiveLiveTranscription(null, false)).toBe(false);
  });
});

// specs/0071 W1 — the chip is labelled with the ACTION, not the state. It used to read
// `Live` / `Deferred`: the owner read "Live" as "make it live", pressed it mid-meeting, and
// the transcript stopped. A button whose two neighbours (Participants, the template picker)
// both do the thing written on them cannot be labelled with its own state.
describe('modeChipDisplay', () => {
  it('offers to PAUSE while transcribing live, with no battery glyph', () => {
    expect(modeChipDisplay(true, false)).toEqual({
      label: 'Pause transcript',
      showBatteryGlyph: false,
    });
    expect(modeChipDisplay(true, true)).toEqual({
      label: 'Pause transcript',
      showBatteryGlyph: false,
    });
  });

  it('offers to RESUME while deferred, with a battery glyph only when on battery', () => {
    expect(modeChipDisplay(false, true)).toEqual({
      label: 'Resume transcript',
      showBatteryGlyph: true,
    });
    expect(modeChipDisplay(false, false)).toEqual({
      label: 'Resume transcript',
      showBatteryGlyph: false,
    });
    expect(modeChipDisplay(false, null)).toEqual({
      label: 'Resume transcript',
      showBatteryGlyph: false,
    });
  });

  // The regression that matters: whatever the label says must be what the click does, so
  // the label must never be the name of the state the session is currently in.
  it('never labels the chip with its own current state', () => {
    for (const live of [true, false]) {
      const { label } = modeChipDisplay(live, false);
      expect(label).not.toBe('Live');
      expect(label).not.toBe('Deferred');
      expect(label.startsWith(live ? 'Pause' : 'Resume')).toBe(true);
    }
  });
});

describe('transcriptEmptyStateVariant', () => {
  it('returns null once live transcription is on or transcripts already exist', () => {
    expect(transcriptEmptyStateVariant(true, false, true)).toBeNull();
    expect(transcriptEmptyStateVariant(false, true, true)).toBeNull();
  });
  it('is the low-power variant when deferred specifically because of battery', () => {
    expect(transcriptEmptyStateVariant(false, false, true)).toBe('low-power-battery');
  });
  it('falls back to the generic variant when deferred for any other reason', () => {
    expect(transcriptEmptyStateVariant(false, false, false)).toBe('live-transcription-off');
    expect(transcriptEmptyStateVariant(false, false, null)).toBe('live-transcription-off');
  });
  it('degrades from the battery variant to the generic one when plugged into AC mid-recording (Finding 4)', () => {
    // Same deferred meeting; only the power source changes (battery → AC).
    expect(transcriptEmptyStateVariant(false, false, true)).toBe('low-power-battery');
    expect(transcriptEmptyStateVariant(false, false, false)).toBe('live-transcription-off');
  });
});

describe('isUnprocessedRecording', () => {
  it('true for a deferred meeting with audio and no transcript', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: true, audioAvailable: true })).toBe(true);
  });
  it('false once a transcript exists', () => {
    expect(isUnprocessedRecording({ hasTranscripts: true, isDeferred: true, audioAvailable: true })).toBe(false);
  });
  it('false when no audio on disk (genuinely empty / notes-only)', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: true, audioAvailable: false })).toBe(false);
  });
  it('false for a normal non-deferred empty view', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: false, audioAvailable: false })).toBe(false);
  });
});
