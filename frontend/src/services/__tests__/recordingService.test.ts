import { describe, it, expect, beforeEach, vi } from 'vitest';

// Regression lock for the recording-start IPC contract. The Rust command
// `start_recording_with_devices_and_meeting` has no `rename_all`, so Tauri
// matches its args by camelCase key. This service historically sent snake_case
// keys (`meeting_name`, …), which silently deserialized to None — the backend
// then minted its own date-stamped meeting name, and the stop path wrote that
// over calendar-event titles. The title must go over the wire as `meetingName`.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { recordingService } from '../recordingService';

describe('recordingService.startRecordingWithDevices', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it('sends the meeting title under the camelCase key the command expects', async () => {
    await recordingService.startRecordingWithDevices(null, null, 'Weekly sync');

    expect(invokeMock).toHaveBeenCalledWith(
      'start_recording_with_devices_and_meeting',
      expect.objectContaining({ meetingName: 'Weekly sync' }),
    );
    const payload = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(payload).not.toHaveProperty('meeting_name');
  });

  it('omits device args so the backend resolves devices from its preference store', async () => {
    // The explicit-devices branch hard-errors when a stored device is gone; the
    // omitted-devices branch falls back preferred -> default. Devices must not
    // ride this invoke under EITHER key convention.
    await recordingService.startRecordingWithDevices(
      'MacBook Pro Microphone (input)',
      'MacBook Pro Speakers (output)',
      'Weekly sync',
    );

    const payload = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(payload).not.toHaveProperty('micDeviceName');
    expect(payload).not.toHaveProperty('mic_device_name');
    expect(payload).not.toHaveProperty('systemDeviceName');
    expect(payload).not.toHaveProperty('system_device_name');
  });
});
