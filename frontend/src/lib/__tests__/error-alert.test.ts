import { describe, it, expect } from 'vitest';
import { splitErrorAlert, DEFAULT_ERROR_ALERT_TITLE } from '@/lib/error-alert';
import { describeRecordingStartError } from '@/lib/recording-start-errors';

describe('splitErrorAlert', () => {
  it('keeps the historical heading when the message has no newline', () => {
    // Every pre-existing producer (transcript-error / transcription-error userMessage) sends
    // one sentence — those must not suddenly lose their sentence into a heading.
    const { title, body } = splitErrorAlert(
      'Recording failed: Unable to initialize speech recognition. Please check your model settings.',
    );
    expect(title).toBe(DEFAULT_ERROR_ALERT_TITLE);
    expect(body).toBe(
      'Recording failed: Unable to initialize speech recognition. Please check your model settings.',
    );
  });

  it('takes the first line as the title and keeps the rest, bullets and all', () => {
    const { title, message } = describeRecordingStartError(new Error('microphone missing'));
    const split = splitErrorAlert(`${title}\n${message}`);
    expect(split.title).toBe('Microphone Not Available');
    expect(split.body.startsWith('Unable to access your microphone')).toBe(true);
    // The bullet lines survive as separate lines — the modal renders them with
    // `whitespace-pre-line`, so the \n separators must still be there.
    expect(split.body.split('\n')).toHaveLength(4);
    expect(split.body).toContain('\n• Your microphone is connected');
  });

  it('falls back to the default heading for empty / blank-first-line input', () => {
    expect(splitErrorAlert('').title).toBe(DEFAULT_ERROR_ALERT_TITLE);
    expect(splitErrorAlert(undefined).body).toBe('');
    const blank = splitErrorAlert('\nsomething went wrong');
    expect(blank.title).toBe(DEFAULT_ERROR_ALERT_TITLE);
    expect(blank.body).toBe('something went wrong');
  });
});
