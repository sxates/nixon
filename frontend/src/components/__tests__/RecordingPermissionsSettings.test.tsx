import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

// spec 0038 WS7.a — the "Recording permissions" control now lives in Settings ›
// General (the former top-level "Permissions" sidebar nav entry is gone). It shows
// mic + audio-capture status and a "Manage permissions" button that opens the
// SAME first-run permissions modal (PermissionsModalContext) — unchanged.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const { openPermissionsModal } = vi.hoisted(() => ({ openPermissionsModal: vi.fn() }));
vi.mock('@/contexts/PermissionsModalContext', () => ({
  usePermissionsModal: () => ({ openPermissionsModal }),
}));

vi.mock('@/hooks/usePlatform', () => ({ useIsLinux: () => false }));

import { RecordingPermissionsSettings } from '../RecordingPermissionsSettings';

function mockBackend({ mic, audioCapture }: { mic: boolean; audioCapture: boolean }) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_audio_devices') {
      return mic ? [{ name: 'Built-in Mic', device_type: 'Input' }] : [];
    }
    if (cmd === 'check_audio_capture_permission_command') {
      return audioCapture;
    }
    return undefined;
  });
}

describe('RecordingPermissionsSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows Allowed for both when mic + audio capture are granted', async () => {
    mockBackend({ mic: true, audioCapture: true });
    render(<RecordingPermissionsSettings />);

    expect(screen.getByText('Microphone')).toBeInTheDocument();
    expect(screen.getByText('Audio capture')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getAllByText('Allowed')).toHaveLength(2);
    });
  });

  it('shows Not granted for a missing permission', async () => {
    mockBackend({ mic: true, audioCapture: false });
    render(<RecordingPermissionsSettings />);

    await waitFor(() => {
      expect(screen.getByText('Not granted')).toBeInTheDocument();
    });
  });

  it('opens the shared permissions modal from "Manage permissions"', async () => {
    mockBackend({ mic: true, audioCapture: true });
    render(<RecordingPermissionsSettings />);

    fireEvent.click(screen.getByRole('button', { name: /manage permissions/i }));
    expect(openPermissionsModal).toHaveBeenCalledTimes(1);
  });
});
