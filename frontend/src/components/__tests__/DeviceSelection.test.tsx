import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// specs/0061 W6 — the "Test Mic" affordance never worked (the button that called it was
// already commented out; `isMonitoring` could never become true). This proves the dead
// monitoring UI and its "Tip: Click Test Mic..." copy are gone, without touching the
// still-live device-selection behavior.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection } from '@/components/DeviceSelection';

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  vi.clearAllMocks();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'get_audio_devices':
        return Promise.resolve([
          { name: 'Built-in Microphone', device_type: 'Input' },
          { name: 'Built-in Output', device_type: 'Output' },
        ]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

describe('DeviceSelection (specs/0061 W6 — dead Test Mic removed)', () => {
  // specs/0066 W1 — the System Audio Backend picker is gone. It did not merely duplicate
  // a default: choosing ScreenCaptureKit routed system audio down the legacy CPAL path
  // (the one that needs BlackHole), and the choice was overwritten on the next load
  // anyway. Capture is pinned to the Core Audio tap.
  it('offers no audio-backend picker', async () => {
    render(
      <DeviceSelection
        selectedDevices={{ micDevice: null, systemDevice: null }}
        onDeviceChange={vi.fn()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Microphone')).toBeInTheDocument());
    expect(screen.queryByText(/backend/i)).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('get_audio_backend_info');
    expect(invokeMock).not.toHaveBeenCalledWith('get_current_audio_backend');
  });

  it('never renders "Test Mic" or "Stop Test" text', async () => {
    render(
      <DeviceSelection
        selectedDevices={{ micDevice: null, systemDevice: null }}
        onDeviceChange={vi.fn()}
      />,
    );

    await waitFor(() => expect(screen.getByText('Microphone')).toBeInTheDocument());

    expect(screen.queryByText(/Test Mic/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Stop Test/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Click .Test Mic./i)).not.toBeInTheDocument();
  });

  it('still lists the real microphone and system-audio devices', async () => {
    render(
      <DeviceSelection
        selectedDevices={{ micDevice: null, systemDevice: null }}
        onDeviceChange={vi.fn()}
      />,
    );

    await waitFor(() => expect(screen.getByText('Microphone')).toBeInTheDocument());
    expect(screen.getByText('System Audio')).toBeInTheDocument();
  });
});
