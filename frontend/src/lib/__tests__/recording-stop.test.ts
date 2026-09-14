import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  requestFullRecordingStop,
  recordStopRecordingResult,
  consumeStopRecordingResult,
  clearStopRecordingResult,
} from '@/lib/recording-stop';

// specs/0024 WS1.1 — Zoom-end (and the global bar) must trigger the FULL two-part stop that
// lives on /record, not the partial `window.handleRecordingStop`. The contract: set the
// on-load flag, navigate to /record, and dispatch the live stop event — so the stop runs
// whether /record is already mounted (event) or about to mount (flag).

describe('requestFullRecordingStop (WS1.1)', () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it('sets the on-load flag so /record runs the stop when it mounts', () => {
    requestFullRecordingStop(() => {});
    expect(sessionStorage.getItem('stopRecordingOnLoad')).toBe('true');
  });

  it('navigates to /record', () => {
    const navigate = vi.fn();
    requestFullRecordingStop(navigate);
    expect(navigate).toHaveBeenCalledWith('/record');
  });

  it('dispatches the live stop event for an already-mounted /record', () => {
    const handler = vi.fn();
    window.addEventListener('stop-recording-from-global-bar', handler);
    requestFullRecordingStop(() => {});
    expect(handler).toHaveBeenCalledTimes(1);
    window.removeEventListener('stop-recording-from-global-bar', handler);
  });

  it('flags before navigating (flag wins the race when /record is not yet mounted)', () => {
    const order: string[] = [];
    const navigate = vi.fn(() => {
      order.push(`flag=${sessionStorage.getItem('stopRecordingOnLoad')}`);
    });
    requestFullRecordingStop(navigate);
    expect(order).toEqual(['flag=true']);
  });
});

// specs/0037 review-2 (FIX C) — the in-memory stash for the `stop_recording` command's
// return payload. It must normalize the backend shape
// { folder_path, meeting_name, resumed, prior_audio_duration_seconds }, tolerate a
// backend that returns nothing (undefined/null → record nothing), and be read-and-clear.
describe('stop-recording result stash (specs/0037 FIX C)', () => {
  beforeEach(() => {
    clearStopRecordingResult();
  });

  it('normalizes and stashes a full backend payload', () => {
    recordStopRecordingResult({
      message: 'Recording stopped',
      folder_path: '/recordings/meeting-a',
      meeting_name: 'Weekly sync',
      resumed: true,
      prior_audio_duration_seconds: 120.5,
      meeting_id: 'meeting-a',
    });

    expect(consumeStopRecordingResult()).toEqual({
      folder_path: '/recordings/meeting-a',
      meeting_name: 'Weekly sync',
      resumed: true,
      prior_audio_duration_seconds: 120.5,
      meeting_id: 'meeting-a',
    });
  });

  it('is read-and-clear: a second consume returns null', () => {
    recordStopRecordingResult({ folder_path: '/r', resumed: false });
    expect(consumeStopRecordingResult()).not.toBeNull();
    expect(consumeStopRecordingResult()).toBeNull();
  });

  it('records nothing for undefined/null (backend without the return-value contract)', () => {
    expect(recordStopRecordingResult(undefined)).toBeNull();
    expect(consumeStopRecordingResult()).toBeNull();
    expect(recordStopRecordingResult(null)).toBeNull();
    expect(consumeStopRecordingResult()).toBeNull();
    expect(recordStopRecordingResult('not-an-object')).toBeNull();
    expect(consumeStopRecordingResult()).toBeNull();
  });

  it('normalizes partial/malformed fields to safe defaults', () => {
    recordStopRecordingResult({
      folder_path: '', // empty → unknown
      meeting_name: 42, // wrong type → unknown
      resumed: 'true', // must be a real boolean true
      prior_audio_duration_seconds: Number.NaN, // non-finite → 0
      meeting_id: 99, // wrong type → null
    });

    expect(consumeStopRecordingResult()).toEqual({
      folder_path: null,
      meeting_name: null,
      resumed: false,
      prior_audio_duration_seconds: 0,
      meeting_id: null,
    });
  });

  it('clearStopRecordingResult drops a stale stash (called at every recording start)', () => {
    recordStopRecordingResult({ folder_path: '/r', resumed: true });
    clearStopRecordingResult();
    expect(consumeStopRecordingResult()).toBeNull();
  });
});
