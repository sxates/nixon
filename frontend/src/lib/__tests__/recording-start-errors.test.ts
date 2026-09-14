import { describe, it, expect } from 'vitest';
import { describeRecordingStartError } from '@/lib/recording-start-errors';

// specs/0057 Plan 2 Task 7 — the device-error mapping extracted verbatim from the retired
// RecordingControls.tsx (:120-146). Same four branches, same titles: the copy is the
// contract the user sees when a start fails.
describe('describeRecordingStartError', () => {
  it('maps microphone/mic/input failures to "Microphone Not Available"', () => {
    for (const raw of ['no microphone found', 'mic busy', 'input device gone']) {
      const { title, message } = describeRecordingStartError(new Error(raw));
      expect(title).toBe('Microphone Not Available');
      expect(message).toContain('Unable to access your microphone');
    }
  });

  it('maps system-audio/speaker/output failures to "System Audio Not Available"', () => {
    for (const raw of ['system audio tap failed', 'speaker unavailable', 'output stream error']) {
      const { title, message } = describeRecordingStartError(new Error(raw));
      expect(title).toBe('System Audio Not Available');
      expect(message).toContain('Unable to capture system audio');
    }
  });

  it('maps permission failures to "Permission Required"', () => {
    const { title, message } = describeRecordingStartError(new Error('permission denied by user'));
    expect(title).toBe('Permission Required');
    expect(message).toContain('Recording permissions are required');
  });

  it('falls back to "Recording Failed" for anything else', () => {
    const { title, message } = describeRecordingStartError(new Error('kaboom'));
    expect(title).toBe('Recording Failed');
    expect(message).toContain('Unable to start recording');
  });

  it('tolerates non-Error inputs: string, object-with-message, null', () => {
    expect(describeRecordingStartError('microphone missing').title).toBe('Microphone Not Available');
    expect(describeRecordingStartError({ message: 'permission denied' }).title).toBe('Permission Required');
    expect(describeRecordingStartError(null).title).toBe('Recording Failed');
    expect(describeRecordingStartError(undefined).title).toBe('Recording Failed');
  });

  // The microphone branch is checked first in the original, so a message naming both
  // keeps the microphone title — preserve that precedence.
  it('keeps the original branch precedence (microphone before system audio)', () => {
    expect(describeRecordingStartError(new Error('microphone and system audio failed')).title).toBe(
      'Microphone Not Available',
    );
  });
});
