import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';

// specs/0061 W3 Task 7 — `trigger_system_audio_permission_command` used to
// return a bare boolean that lied: tap construction succeeds even when
// Audio Capture permission is denied (the tap just delivers silence), so a
// user could finish onboarding believing system audio worked when it did
// not. The backend now probes the tap with a real tone and reports
// granted/silent/failed. This test covers the `silent` branch: the row must
// show the exact onboarding copy, with no toast, and Continue/Skip must
// remain reachable — silent is an honest "not yet", not a dead end.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const toastErrorMock = vi.fn();
vi.mock('sonner', () => ({ toast: { error: (...args: unknown[]) => toastErrorMock(...args) } }));

const setPermissionStatusMock = vi.fn();
const setPermissionsSkippedMock = vi.fn();
const goNextMock = vi.fn();

const basePermissions = {
  microphone: 'authorized' as const,
  systemAudio: 'not_determined' as const,
  screenRecording: 'not_determined' as const,
};

vi.mock('@/contexts/OnboardingContext', () => ({
  // A real `useState` here (not a plain mock object) so that calling
  // `setPermissionStatus` actually re-renders `PermissionsStep` with the
  // updated status, the same way the real context does.
  useOnboarding: () => {
    const [permissions, setPermissions] = React.useState(basePermissions);
    return {
      permissions,
      setPermissionStatus: (key: 'microphone' | 'systemAudio' | 'screenRecording', status: string) => {
        setPermissionStatusMock(key, status);
        setPermissions((prev) => ({ ...prev, [key]: status }));
      },
      setPermissionsSkipped: setPermissionsSkippedMock,
      goNext: goNextMock,
    };
  },
}));

import { PermissionsStep } from '@/components/onboarding/steps/PermissionsStep';

beforeEach(() => {
  invokeMock.mockReset();
  toastErrorMock.mockClear();
  setPermissionStatusMock.mockClear();
  setPermissionsSkippedMock.mockClear();
  goNextMock.mockClear();
});

describe('PermissionsStep — honest Audio Capture probe (specs/0061 W3)', () => {
  it('shows the exact silent copy (no toast) and keeps Continue/skip reachable when the probe reports silent', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'trigger_system_audio_permission_command') {
        return { state: 'silent' };
      }
      return undefined;
    });

    render(<PermissionsStep />);

    // Microphone is already 'authorized' in the base fixture, so this is
    // unambiguously the System Audio row's button.
    fireEvent.click(screen.getByRole('button', { name: /enable/i }));

    await screen.findByText('Not yet — macOS may ask again when you first record.');

    expect(setPermissionStatusMock).toHaveBeenCalledWith('systemAudio', 'silent');
    expect(toastErrorMock).not.toHaveBeenCalled();

    // Continue is still disabled (silent isn't a grant)...
    expect(screen.getByRole('button', { name: /continue/i })).toBeDisabled();

    // ...but skip is always reachable, regardless of permission state.
    const skipButton = screen.getByText("I'll do this later");
    expect(skipButton).toBeInTheDocument();
    fireEvent.click(skipButton);
    expect(setPermissionsSkippedMock).toHaveBeenCalledWith(true);
    expect(goNextMock).toHaveBeenCalled();
  });
});
