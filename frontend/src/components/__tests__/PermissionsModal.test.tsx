import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

// The "Enable recording" modal. Once a row is denied, its button becomes
// "Open Settings" and must deep-link that permission's Privacy pane. It used to
// call `open_system_settings` with no `preferencePane` (which the Rust command
// requires), so the invoke rejected and System Settings never opened.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock('@/lib/calendar', () => ({
  getCalendarAccessStatus: vi.fn().mockResolvedValue('authorized'),
  requestCalendarAccess: vi.fn().mockResolvedValue(true),
}));

const closeMock = vi.fn();
vi.mock('@/contexts/PermissionsModalContext', () => ({
  usePermissionsModal: () => ({
    isOpen: true,
    setOpen: vi.fn(),
    closePermissionsModal: closeMock,
    openPermissionsModal: vi.fn(),
  }),
}));

const recordingToggleMock = vi.fn();
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: recordingToggleMock }),
}));

import PermissionsModal from '@/components/PermissionsModal';

/** The PermissionRow whose title is `title` (its root carries `data-state`). */
function row(title: string): HTMLElement {
  const el = screen.getByText(title).closest('[data-state]') as HTMLElement | null;
  if (!el) throw new Error(`no row for ${title}`);
  return el;
}

beforeEach(() => {
  invokeMock.mockReset();
  closeMock.mockClear();
  recordingToggleMock.mockClear();
});

describe('PermissionsModal — Open Settings deep-links the right pane', () => {
  it('denied microphone → Privacy_Microphone', async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'trigger_microphone_permission' ? false : undefined,
    );
    render(<PermissionsModal />);
    fireEvent.click(within(row('Microphone')).getByRole('button', { name: 'Allow' }));
    fireEvent.click(await within(row('Microphone')).findByRole('button', { name: 'Open Settings' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_system_settings', {
        preferencePane: 'Privacy_Microphone',
      }),
    );
  });

  it('denied audio capture → Privacy_ScreenCapture', async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'trigger_system_audio_permission_command' ? { state: 'failed' } : undefined,
    );
    render(<PermissionsModal />);
    fireEvent.click(within(row('Audio capture')).getByRole('button', { name: 'Allow' }));
    fireEvent.click(
      await within(row('Audio capture')).findByRole('button', { name: 'Open Settings' }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('open_system_settings', {
        preferencePane: 'Privacy_ScreenCapture',
      }),
    );
  });

  it('"Start recording" appears once both are granted and starts a recording', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'trigger_microphone_permission') return true;
      if (cmd === 'trigger_system_audio_permission_command') return { state: 'granted' };
      return undefined;
    });
    render(<PermissionsModal />);
    fireEvent.click(within(row('Microphone')).getByRole('button', { name: 'Allow' }));
    await waitFor(() =>
      expect(within(row('Microphone')).queryByRole('button', { name: 'Allow' })).toBeNull(),
    );
    fireEvent.click(within(row('Audio capture')).getByRole('button', { name: 'Allow' }));
    fireEvent.click(await screen.findByRole('button', { name: /start recording/i }));
    expect(closeMock).toHaveBeenCalled();
    expect(recordingToggleMock).toHaveBeenCalled();
  });
});
