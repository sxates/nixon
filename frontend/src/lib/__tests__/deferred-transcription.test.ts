import { describe, it, expect } from 'vitest';
import {
  needsTranscriptionBeforeSummary,
  retranscriptionProviderFor,
  SPARSE_TRANSCRIPT_SEGMENTS,
} from '@/lib/deferred-transcription';

// specs/0029 WS7.2 — record-only mode's deferred-transcription detection. The summary
// flow (useSummaryGeneration) runs transcription first ONLY when the transcript is
// absent/sparse AND audio is positively known to exist on disk.

describe('needsTranscriptionBeforeSummary (WS7.2)', () => {
  it('transcribes a transcript-empty meeting with audio', () => {
    expect(needsTranscriptionBeforeSummary(0, true)).toBe(true);
  });

  it('transcribes a sparse-transcript meeting with audio', () => {
    expect(needsTranscriptionBeforeSummary(SPARSE_TRANSCRIPT_SEGMENTS - 1, true)).toBe(true);
  });

  it('does NOT transcribe once the transcript reaches the sparse threshold', () => {
    expect(needsTranscriptionBeforeSummary(SPARSE_TRANSCRIPT_SEGMENTS, true)).toBe(false);
    expect(needsTranscriptionBeforeSummary(500, true)).toBe(false);
  });

  it('does NOT transcribe when audio is known to be gone (retention sweep)', () => {
    expect(needsTranscriptionBeforeSummary(0, false)).toBe(false);
  });

  it('does NOT transcribe when the audio probe is pending/failed (null)', () => {
    // Probe failures must leave the pre-existing summary behavior untouched
    // (notes-grounded fallback or friendly bail-out) — never a doomed transcription.
    expect(needsTranscriptionBeforeSummary(0, null)).toBe(false);
  });

  it('threshold mirrors the backend retention exemption (3 segments)', () => {
    // retention::MIN_TRANSCRIPT_SEGMENTS keeps exactly these meetings' audio
    // from being deleted so it can still be transcribed later.
    expect(SPARSE_TRANSCRIPT_SEGMENTS).toBe(3);
  });
});

describe('retranscriptionProviderFor (WS7.2)', () => {
  it("maps config 'localWhisper' to backend 'whisper'", () => {
    expect(retranscriptionProviderFor('localWhisper')).toBe('whisper');
  });

  it("passes 'whisper' and 'parakeet' through", () => {
    expect(retranscriptionProviderFor('whisper')).toBe('whisper');
    expect(retranscriptionProviderFor('parakeet')).toBe('parakeet');
  });

  it('returns null (backend default) for unknown/missing providers', () => {
    expect(retranscriptionProviderFor('deepgram')).toBeNull();
    expect(retranscriptionProviderFor(undefined)).toBeNull();
    expect(retranscriptionProviderFor(null)).toBeNull();
  });
});
