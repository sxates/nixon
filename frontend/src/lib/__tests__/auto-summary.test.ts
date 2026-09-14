import { describe, it, expect } from 'vitest';
import {
  AutoSummaryGateInput,
  autoSummarySkipReason,
  autoSummarySkipToast,
} from '@/lib/auto-summary';

// specs/0029 WS7.3 — the auto-summary gate on /meeting-details?source=recording. Every
// skip must have an explicit reason (surfaced as a toast when the user just finished a
// recording), and normal navigation must stay silent.

const passing: AutoSummaryGateInput = {
  source: 'recording',
  isAutoSummaryEnabled: true,
  transcriptCount: 12,
  hasModelConfigured: true,
};

describe('autoSummarySkipReason (WS7.3)', () => {
  it('proceeds (null) when all gates pass', () => {
    expect(autoSummarySkipReason(passing)).toBeNull();
  });

  it('skips silently-classified when not navigated from a recording', () => {
    expect(autoSummarySkipReason({ ...passing, source: null })).toBe('not-from-recording');
    expect(autoSummarySkipReason({ ...passing, source: 'sidebar' })).toBe('not-from-recording');
  });

  it('proceeds when navigated from an audio import (source=import)', () => {
    // Imports land here with a full transcript already — treat them like recordings.
    expect(autoSummarySkipReason({ ...passing, source: 'import' })).toBeNull();
  });

  it('skips when the toggle is off (the default!)', () => {
    expect(
      autoSummarySkipReason({ ...passing, isAutoSummaryEnabled: false }),
    ).toBe('disabled');
  });

  it('skips when the transcript is empty', () => {
    expect(autoSummarySkipReason({ ...passing, transcriptCount: 0 })).toBe('empty-transcript');
  });

  // specs/0029 WS7.2: record-only meetings have no transcript but DO have audio
  // awaiting deferred transcription — auto-summary proceeds (transcribe-then-summarize).
  it('proceeds on an empty transcript when audio awaits deferred transcription', () => {
    expect(
      autoSummarySkipReason({
        ...passing,
        transcriptCount: 0,
        audioAwaitingTranscription: true,
      }),
    ).toBeNull();
  });

  it('still skips an empty transcript when audio is NOT awaiting transcription', () => {
    expect(
      autoSummarySkipReason({
        ...passing,
        transcriptCount: 0,
        audioAwaitingTranscription: false,
      }),
    ).toBe('empty-transcript');
  });

  it('skips when no model is configured and no fallback exists', () => {
    expect(
      autoSummarySkipReason({ ...passing, hasModelConfigured: false }),
    ).toBe('no-model');
  });

  // 1.10 feedback: a battery-deferred meeting must run NOTHING automatically at
  // stop — landing on /meeting-details?source=recording was kicking off the full
  // transcribe-then-summarize pass immediately (and, since it never cleared the
  // defer marker, the AC backlog then re-ran everything a second time).
  it('skips a deferred meeting (processing_mode=defer) — backlog/manual owns it', () => {
    expect(
      autoSummarySkipReason({
        ...passing,
        transcriptCount: 0,
        audioAwaitingTranscription: true,
        processingMode: 'defer',
      }),
    ).toBe('deferred');
  });

  it('skips (silently) a mid-meeting go-live meeting whose immediate pass is running', () => {
    // stopAction 'process-now' dispatches the full pipeline via the backlog; the
    // page visit must not start a SECOND summary in parallel.
    expect(
      autoSummarySkipReason({
        ...passing,
        transcriptCount: 5,
        processingMode: 'live',
      }),
    ).toBe('deferred-processing');
  });

  // spec 0051 final review, Finding 1: the SUCCESS path of a 'process-now' stop marks
  // the meeting 'defer' and hands it to the backlog, which clears the marker only at the
  // END of a multi-minute pipeline. The post-stop visit lands ~2s later, so 'defer' alone
  // would toast "this meeting still needs processing" WHILE the backlog pill shows it
  // processing. In-flight in the backlog ⇒ silent 'deferred-processing'.
  it('skips SILENTLY when the backlog already has this defer-marked meeting in flight', () => {
    const reason = autoSummarySkipReason({
      ...passing,
      processingMode: 'defer',
      isProcessingInBacklog: true,
    });
    expect(reason).toBe('deferred-processing');
    expect(autoSummarySkipToast(reason!)).toBeNull();
  });

  it('still toasts "Processing deferred" for a defer-marked meeting NOT in the backlog', () => {
    // The genuinely-waiting case (battery-deferred, or a handoff that never landed):
    // the user must be told it needs processing.
    const reason = autoSummarySkipReason({
      ...passing,
      processingMode: 'defer',
      isProcessingInBacklog: false,
    });
    expect(reason).toBe('deferred');
    expect(autoSummarySkipToast(reason!)).not.toBeNull();
  });

  it('treats an absent isProcessingInBacklog as "not in flight"', () => {
    expect(autoSummarySkipReason({ ...passing, processingMode: 'defer' })).toBe('deferred');
  });

  it('deferral outranks the disabled toggle and transcript/model gates', () => {
    expect(
      autoSummarySkipReason({
        ...passing,
        isAutoSummaryEnabled: false,
        transcriptCount: 0,
        hasModelConfigured: false,
        processingMode: 'defer',
      }),
    ).toBe('deferred');
  });

  it('treats an absent/null processing mode exactly as before', () => {
    expect(autoSummarySkipReason({ ...passing, processingMode: null })).toBeNull();
    expect(autoSummarySkipReason({ ...passing, processingMode: undefined })).toBeNull();
  });

  it('prioritizes source over every other gate (never toast on normal navigation)', () => {
    expect(
      autoSummarySkipReason({
        source: 'sidebar',
        isAutoSummaryEnabled: false,
        transcriptCount: 0,
        hasModelConfigured: false,
      }),
    ).toBe('not-from-recording');
  });

  it('prioritizes the disabled toggle over transcript/model state', () => {
    // The most actionable reason first: the user asked for auto-summaries without
    // knowing the switch exists — tell them about the switch.
    expect(
      autoSummarySkipReason({
        ...passing,
        isAutoSummaryEnabled: false,
        transcriptCount: 0,
        hasModelConfigured: false,
      }),
    ).toBe('disabled');
  });

  it('prioritizes empty transcript over missing model', () => {
    expect(
      autoSummarySkipReason({ ...passing, transcriptCount: 0, hasModelConfigured: false }),
    ).toBe('empty-transcript');
  });
});

// spec 0051 WS2 — 'defer' now means "needs processing" generally, not just "recorded
// in low-power mode": the stop path writes it for a mid-meeting go-live too. The copy
// must not claim low-power mode.
describe('autoSummarySkipToast — 0051 defer copy', () => {
  it('explains deferral without asserting low-power mode', () => {
    const toast = autoSummarySkipToast('deferred');
    expect(toast).not.toBeNull();
    expect(toast!.description).not.toMatch(/low[- ]power/i);
    expect(toast!.description).toMatch(/Process now/i);
  });
});

describe('autoSummarySkipToast (WS7.3)', () => {
  it('stays silent for normal navigation', () => {
    expect(autoSummarySkipToast('not-from-recording')).toBeNull();
  });

  it('explains the deferral and how to process manually', () => {
    const copy = autoSummarySkipToast('deferred');
    expect(copy?.title).toContain('deferred');
    expect(copy?.description).toContain('Process now');
  });

  it('stays silent when the immediate go-live pass is already running', () => {
    // The backlog banner already shows that pipeline's progress — no extra toast.
    expect(autoSummarySkipToast('deferred-processing')).toBeNull();
  });

  it('tells the user where the toggle lives when auto-summary is off', () => {
    const copy = autoSummarySkipToast('disabled');
    expect(copy?.title).toBe('Auto-summary is off');
    expect(copy?.description).toContain('Summarize automatically when a meeting ends');
  });

  it('has user-facing copy for every non-silent reason', () => {
    for (const reason of ['disabled', 'empty-transcript', 'no-model', 'model-check-failed'] as const) {
      const copy = autoSummarySkipToast(reason);
      expect(copy).not.toBeNull();
      expect(copy!.title.length).toBeGreaterThan(0);
    }
  });
});
