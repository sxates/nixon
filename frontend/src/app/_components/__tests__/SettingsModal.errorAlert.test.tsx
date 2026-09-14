import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => ({ modelConfig: {}, models: [], modelOptions: [], selectedDevices: {}, transcriptModelConfig: {}, setModelConfig: vi.fn(), setSelectedDevices: vi.fn(), setTranscriptModelConfig: vi.fn(), toggleConfidenceIndicator: vi.fn() }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording: false }) }));
vi.mock('@/components/PreferenceSettings', () => ({ PreferenceSettings: () => null }));
vi.mock('@/components/DeviceSelection', () => ({ DeviceSelection: () => null }));
vi.mock('@/components/TranscriptSettings', () => ({ TranscriptSettings: () => null }));
vi.mock('@/components/ModelSettingsModal', () => ({}));

import { SettingsModals } from '@/app/_components/SettingsModal';

const modals = { modelSettings: false, deviceSettings: false, modelSelector: false, errorAlert: true, chunkDropWarning: false };

function renderAlert(message: string) {
  return render(
    <SettingsModals
      modals={modals}
      messages={{ errorAlert: message, chunkDropWarning: '', modelSelector: '' }}
      onClose={vi.fn()}
    />,
  );
}

// specs/0057 Plan 2 Task 7 (review round 1): the device-error copy has to RENDER as copy —
// its own heading, and its bullet list on separate lines.
describe('SettingsModals — error alert', () => {
  it('promotes a leading title line and keeps the bullet lines pre-formatted', () => {
    const { container } = renderAlert(
      'Microphone Not Available\nUnable to access your microphone. Please check that:\n• Your microphone is connected\n• The app has microphone permissions',
    );
    expect(screen.getByText('Microphone Not Available')).toBeTruthy();
    const body = container.querySelector('.whitespace-pre-line');
    expect(body).toBeTruthy();
    expect(body!.textContent).toContain('• Your microphone is connected');
    expect(body!.textContent).not.toContain('Microphone Not Available');
  });

  it('keeps the historical heading for a single-sentence message (stop path unchanged)', () => {
    renderAlert('Recording failed: Unable to initialize speech recognition.');
    expect(screen.getByText('Recording Stopped')).toBeTruthy();
    expect(screen.getByText(/Unable to initialize speech recognition/)).toBeTruthy();
  });
});
