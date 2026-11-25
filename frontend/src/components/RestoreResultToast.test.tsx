import { describe, it, expect, vi, beforeEach } from 'vitest';
import { StrictMode } from 'react';
import { render, waitFor } from '@testing-library/react';

const { invoke, toast } = vi.hoisted(() => ({
  invoke: vi.fn(),
  toast: { success: vi.fn(), error: vi.fn() },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('sonner', () => ({ toast }));

import { RestoreResultToast, resetRestoreResultAsked } from '@/components/RestoreResultToast';

beforeEach(() => {
  resetRestoreResultAsked();
  invoke.mockReset();
  toast.success.mockReset();
  toast.error.mockReset();
});

describe('RestoreResultToast', () => {
  it('reads the result once under strict mode and shows it', async () => {
    invoke.mockResolvedValue({ ok: true, meetings: 42, createdAt: '2026-11-24T10:00:00Z' });
    render(
      <StrictMode>
        <RestoreResultToast />
      </StrictMode>,
    );
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith('Restored 42 meetings from Nov 24, 2026.'));
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('shows a failed restore as an error', async () => {
    invoke.mockResolvedValue({ ok: false, meetings: 0, createdAt: '2026-11-24T10:00:00Z', error: 'disk full' });
    render(<RestoreResultToast />);
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Restore failed: disk full', expect.anything()),
    );
  });

  it('shows nothing when there is no result', async () => {
    invoke.mockResolvedValue(null);
    render(<RestoreResultToast />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.error).not.toHaveBeenCalled();
  });
});
