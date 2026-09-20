import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';

// specs/0067 (owner request) — Settings → Audio replaces the Permissions tab and the
// Recording tab's "Audio devices" section. Permission and device sit on the same row per
// input, because when the microphone is not working you do not know in advance which of
// the two is wrong. Carries over the guarantees of the old RecordingPermissionsSettings:
// status is read from the backend, and granting goes through the same first-run modal.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const { openPermissionsModal } = vi.hoisted(() => ({ openPermissionsModal: vi.fn() }));
vi.mock('@/contexts/PermissionsModalContext', () => ({
  usePermissionsModal: () => ({ openPermissionsModal }),
}));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { AudioSettings } from '../AudioSettings';

function mockBackend({
  mic,
  audioCapture,
  micDevice = null,
}: {
  mic: boolean;
  audioCapture: boolean;
  micDevice?: string | null;
}) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_audio_devices') {
      return [
        ...(mic ? [{ name: 'Built-in Mic', device_type: 'Input' }] : []),
        { name: 'Built-in Output', device_type: 'Output' },
      ];
    }
    if (cmd === 'check_audio_capture_permission_command') return audioCapture;
    if (cmd === 'get_recording_preferences') {
      return { preferred_mic_device: micDevice, preferred_system_device: null };
    }
    return undefined;
  });
}

const row = (key: 'mic' | 'system') => screen.getByTestId(`audio-row-${key}`);

beforeEach(() => {
  vi.clearAllMocks();
});

describe('AudioSettings', () => {
  it('shows permission and device on one row, per input', async () => {
    mockBackend({ mic: true, audioCapture: true, micDevice: 'Built-in Mic' });
    render(<AudioSettings />);

    await waitFor(() => expect(within(row('mic')).getByText('Allowed')).toBeInTheDocument());
    expect(within(row('mic')).getByLabelText('Microphone device')).toBeInTheDocument();

    expect(within(row('system')).getByText('Allowed')).toBeInTheDocument();
    expect(within(row('system')).getByLabelText('System audio device')).toBeInTheDocument();
  });

  it('offers the guided modal — not a second grant path — when a permission is missing', async () => {
    mockBackend({ mic: false, audioCapture: false });
    render(<AudioSettings />);

    const allow = await within(row('system')).findByRole('button', { name: /allow/i });
    fireEvent.click(allow);

    expect(openPermissionsModal).toHaveBeenCalled();
  });

  it('reports the two permissions independently', async () => {
    mockBackend({ mic: true, audioCapture: false });
    render(<AudioSettings />);

    await waitFor(() => expect(within(row('mic')).getByText('Allowed')).toBeInTheDocument());
    expect(within(row('system')).queryByText('Allowed')).toBeNull();
  });

  it('reads the saved device choice rather than always showing the default', async () => {
    mockBackend({ mic: true, audioCapture: true, micDevice: 'Built-in Mic' });
    render(<AudioSettings />);

    await waitFor(() =>
      expect(within(row('mic')).getByLabelText('Microphone device')).toHaveTextContent('Built-in Mic'),
    );
  });

  // The Recording tab writes the same preferences store, so a whole-object save from
  // either side would revert the other's change. Both go through a read-modify-write.
  it('saves a device change by patching, never by overwriting the store', async () => {
    mockBackend({ mic: true, audioCapture: true });
    render(<AudioSettings />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('get_recording_preferences'));

    invoke.mockClear();
    mockBackend({ mic: true, audioCapture: true });
    fireEvent.click(within(row('mic')).getByLabelText('Microphone device'));
    const option = await screen.findByRole('option', { name: 'Built-in Mic' });
    fireEvent.click(option);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_recording_preferences', expect.anything()));
    // The write is preceded by a read of what is currently stored.
    const order = invoke.mock.calls.map(([cmd]) => cmd);
    expect(order.indexOf('get_recording_preferences')).toBeLessThan(
      order.indexOf('set_recording_preferences'),
    );
  });
});
