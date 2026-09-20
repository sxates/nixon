import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { AddMeetingDialog } from '@/components/Today/AddMeetingDialog';

describe('AddMeetingDialog (specs/0069 W3)', () => {
  beforeEach(() => invoke.mockReset());

  it('creates a meeting at the chosen local time', async () => {
    invoke.mockResolvedValue('meeting-1');
    const onSaved = vi.fn();
    render(
      <AddMeetingDialog open onOpenChange={() => {}} defaultDateKey="2026-09-20" onSaved={onSaved} />,
    );
    fireEvent.change(screen.getByLabelText(/title/i), { target: { value: 'Call with Sam' } });
    fireEvent.change(screen.getByLabelText(/start/i), { target: { value: '15:00' } });
    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_create_manual_meeting', expect.anything()));
    const [, args] = invoke.mock.calls[0] as [string, Record<string, unknown>];
    expect(args.title).toBe('Call with Sam');
    // Local 15:00 on the viewed day, sent as an absolute instant.
    expect(new Date(args.startsAt as string).getHours()).toBe(15);
    // Default duration is 30 minutes.
    expect(
      (new Date(args.endsAt as string).getTime() - new Date(args.startsAt as string).getTime()) / 60000,
    ).toBe(30);
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
  });

  it('will not submit an empty title', () => {
    render(<AddMeetingDialog open onOpenChange={() => {}} defaultDateKey="2026-09-20" onSaved={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));
    expect(invoke).not.toHaveBeenCalled();
  });

  it('updates instead of creating when editing', async () => {
    invoke.mockResolvedValue(undefined);
    render(
      <AddMeetingDialog
        open
        onOpenChange={() => {}}
        defaultDateKey="2026-09-20"
        editing={{
          meetingId: 'meeting-9',
          title: 'Call with Sam',
          startsAt: '2026-09-20T15:00:00.000Z',
          endsAt: '2026-09-20T15:30:00.000Z',
          joinUrl: null,
        }}
        onSaved={vi.fn()}
      />,
    );
    expect(screen.getByLabelText(/title/i)).toHaveValue('Call with Sam');
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        'api_update_manual_meeting',
        expect.objectContaining({ meetingId: 'meeting-9' }),
      ),
    );
  });
});
