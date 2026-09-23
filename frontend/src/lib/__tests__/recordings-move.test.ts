import { describe, it, expect } from 'vitest';
import {
  finishToast,
  formatBytes,
  isGatherState,
  isICloudDrivePath,
  isMoveStatus,
  progressLine,
  progressPercent,
  type MoveStatus,
} from '@/lib/recordings-move';

const status = (over: Partial<MoveStatus> = {}): MoveStatus => ({
  done: 0,
  total: 4,
  bytesDone: 0,
  bytesTotal: 0,
  currentTitle: null,
  waitingFor: null,
  target: '/t',
  ...over,
});

describe('recordings-move copy (specs/0073)', () => {
  it('counts the meeting in flight, and never past the total', () => {
    expect(progressLine(status({ done: 0, currentTitle: 'Standup' }))).toBe('Moving 1 of 4 — Standup');
    expect(progressLine(status({ done: 4 }))).toBe('Moving 4 of 4');
  });

  it('says what the move is waiting for', () => {
    expect(progressLine(status({ waitingFor: 'diarization' }))).toBe(
      'Waiting for speaker identification to finish…',
    );
  });

  it('measures progress in bytes when copying, else in meetings', () => {
    expect(progressPercent(status({ bytesDone: 1, bytesTotal: 4, done: 3 }))).toBe(25);
    expect(progressPercent(status({ done: 2 }))).toBe(50);
    expect(progressPercent(status({ total: 0 }))).toBe(0);
  });

  it('builds the finish toast', () => {
    expect(finishToast({ moved: 1, skipped: 0, failed: [], cancelled: false, removedRoots: [] })).toEqual({
      kind: 'success',
      title: 'Moved 1 recording',
    });
    expect(
      finishToast({ moved: 5, skipped: 0, failed: [], cancelled: true, removedRoots: [] })?.title,
    ).toBe('Stopped after moving 5 recordings');
  });

  it('formats sizes in decimal units', () => {
    expect(formatBytes(12.34e9)).toBe('12.3 GB');
    expect(formatBytes(250e6)).toBe('250 MB');
    expect(formatBytes(10)).toBe('less than 1 MB');
  });

  it('rejects payloads it does not understand', () => {
    expect(isMoveStatus({})).toBe(false);
    expect(isMoveStatus([])).toBe(false);
    expect(isGatherState({ needsConfirmation: true, plan: {} })).toBe(false);
  });

  it('recognises iCloud Drive', () => {
    expect(isICloudDrivePath('/Users/a/Library/Mobile Documents/com~apple~CloudDocs/Rec')).toBe(true);
    expect(isICloudDrivePath('/Users/a/Movies/Rec')).toBe(false);
  });
});
