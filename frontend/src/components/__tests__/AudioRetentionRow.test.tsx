import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useState } from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import type { RecordingPreferences } from '@/components/RecordingSettings';
import type { RetentionPreview, RetentionReport } from '@/lib/audio-retention';

// specs/0072 W3 task 23 — shortening retention asks first (count + size), Cancel saves
// nothing, Delete audio saves, deletes now and toasts the REPORTED numbers. Keeping audio
// longer, or a change that deletes nothing, opens no dialog.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));
// Radix Select doesn't open under jsdom; a native select exercises the same onValueChange.
vi.mock('@/components/ui/select', () => ({
  Select: ({
    value,
    onValueChange,
    disabled,
    children,
  }: {
    value: string;
    onValueChange: (v: string) => void;
    disabled?: boolean;
    children: React.ReactNode;
  }) => (
    <select
      aria-label="Delete audio recordings"
      value={value}
      disabled={disabled}
      onChange={(e) => onValueChange(e.target.value)}
    >
      {children}
    </select>
  ),
  SelectTrigger: () => null,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SelectItem: ({ value, children }: { value: string; children: React.ReactNode }) => (
    <option value={value}>{children}</option>
  ),
}));

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AudioRetentionRow } from '@/components/AudioRetentionRow';

const invokeMock = vi.mocked(invoke);

const PREFS: RecordingPreferences = {
  save_folder: '/tmp/rec',
  auto_save: true,
  audio_retention: { mode: 'days', days: 30 },
  preferred_mic_device: null,
  preferred_system_device: null,
  retention_days: 30,
  live_transcription_enabled: true,
  low_power_on_battery: true,
};

let preview: RetentionPreview;
let report: RetentionReport;

function Harness() {
  const [prefs, setPrefs] = useState(PREFS);
  return <AudioRetentionRow preferences={prefs} setPreferences={setPrefs} />;
}

const select = () => screen.getByLabelText('Delete audio recordings') as HTMLSelectElement;
const called = (cmd: string) => invokeMock.mock.calls.some(([c]) => c === cmd);

beforeEach(() => {
  vi.clearAllMocks();
  preview = { meetings: 23, bytes: 4.1e9, keptPending: 3, keptFailed: 0, busy: 0 };
  report = { meetingsPurged: 23, bytesFreed: 4.1e9, skippedBusy: 0 };
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_preview_audio_retention':
        return preview;
      case 'api_apply_audio_retention_now':
        return report;
      case 'get_recording_preferences':
        return PREFS;
      default:
        return undefined;
    }
  });
});

describe('AudioRetentionRow — confirm before deleting (specs/0072 task 23)', () => {
  it('lowering with meetings to delete previews the candidate and opens the dialog with count and size', async () => {
    render(<Harness />);
    fireEvent.change(select(), { target: { value: '7' } });

    expect(
      await screen.findByText('Delete audio from 23 meetings (4.1 GB) now?'),
    ).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('api_preview_audio_retention', {
      policy: { mode: 'days', days: 7 },
    });
    expect(screen.getByText(/older than 7 days will be deleted/)).toBeInTheDocument();
    expect(
      screen.getByText('3 meetings still being processed keep their audio until they finish.'),
    ).toBeInTheDocument();
    // Asked, not done: nothing saved, nothing deleted.
    expect(called('set_recording_preferences')).toBe(false);
    expect(called('api_apply_audio_retention_now')).toBe(false);
  });

  it('Cancel reverts the select and saves nothing', async () => {
    render(<Harness />);
    fireEvent.change(select(), { target: { value: '7' } });
    await screen.findByText('Delete audio from 23 meetings (4.1 GB) now?');

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() =>
      expect(screen.queryByText('Delete audio from 23 meetings (4.1 GB) now?')).toBeNull(),
    );
    expect(select().value).toBe('30');
    expect(called('set_recording_preferences')).toBe(false);
    expect(called('api_apply_audio_retention_now')).toBe(false);
  });

  it('Delete audio saves the new policy, then deletes, then toasts the reported result', async () => {
    report = { meetingsPurged: 21, bytesFreed: 3.8e9, skippedBusy: 2 };
    render(<Harness />);
    fireEvent.change(select(), { target: { value: '7' } });
    await screen.findByText('Delete audio from 23 meetings (4.1 GB) now?');

    fireEvent.click(screen.getByRole('button', { name: 'Delete audio' }));

    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith('Deleted audio from 21 meetings, 3.8 GB freed', {
        description: '2 were busy and will be deleted shortly.',
      }),
    );
    const cmds = invokeMock.mock.calls.map(([c]) => c);
    // Saved BEFORE the apply: apply runs the saved policy.
    expect(cmds.indexOf('set_recording_preferences')).toBeLessThan(
      cmds.indexOf('api_apply_audio_retention_now'),
    );
    expect(invokeMock).toHaveBeenCalledWith('set_recording_preferences', {
      preferences: expect.objectContaining({
        audio_retention: { mode: 'days', days: 7 },
        auto_save: true,
        retention_days: 7,
      }),
    });
    expect(select().value).toBe('7');
  });

  it('keeping audio longer saves without a preview or a dialog', async () => {
    render(<Harness />);
    fireEvent.change(select(), { target: { value: '90' } });

    await waitFor(() => expect(called('set_recording_preferences')).toBe(true));
    expect(called('api_preview_audio_retention')).toBe(false);
    expect(called('api_apply_audio_retention_now')).toBe(false);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(toast.success).toHaveBeenCalledWith('Preference saved', {
      description: 'Audio from meetings older than 90 days will be deleted.',
    });
  });

  it('lowering when nothing would be deleted saves without a dialog and states the new rule', async () => {
    preview = { meetings: 0, bytes: 0, keptPending: 2, keptFailed: 0, busy: 0 };
    render(<Harness />);
    fireEvent.change(select(), { target: { value: 'once-processed' } });

    await waitFor(() => expect(called('set_recording_preferences')).toBe(true));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(called('api_apply_audio_retention_now')).toBe(false);
    expect(toast.success).toHaveBeenCalledWith('Preference saved', {
      description:
        'Audio will be deleted once each meeting is transcribed and its speakers are identified.',
    });
  });

  it('a failed preview saves nothing', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_preview_audio_retention') throw new Error('db locked');
      return undefined;
    });
    render(<Harness />);
    fireEvent.change(select(), { target: { value: '7' } });

    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(called('set_recording_preferences')).toBe(false);
    expect(select().value).toBe('30');
  });
});
