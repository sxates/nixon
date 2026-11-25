import { describe, expect, it } from 'vitest';
import { backupErrorText } from './backup';

describe('backupErrorText', () => {
  it('shows the recording-active copy without a file-error prefix', () => {
    expect(
      backupErrorText({
        kind: 'recordingActive',
        message: 'Stop the recording before replacing your data.',
      }),
    ).toBe('Stop the recording before replacing your data.');
  });
});
