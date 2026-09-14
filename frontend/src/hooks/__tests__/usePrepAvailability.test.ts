import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));

import { usePrepAvailability } from '@/hooks/usePrepAvailability';

const view = (over: Record<string, unknown> = {}) => ({
  meetingId: 'meeting-1',
  origin: 'recorded',
  title: 'Weekly Sync',
  briefStatus: 'absent',
  briefMarkdown: null,
  briefSources: [],
  openItems: [],
  prepNotesMarkdown: null,
  prepNotesJson: null,
  linkedMeetings: [],
  ...over,
});

describe('usePrepAvailability (specs/0054 W4)', () => {
  beforeEach(() => invokeMock.mockReset());

  it('reports no prep for an ad-hoc recording, so the button stays hidden', async () => {
    invokeMock.mockResolvedValue(view());
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(result.current.hasPrep).toBe(false);
    expect(result.current.openItemCount).toBe(0);
  });

  it('does not treat the "absent" brief status as prep', async () => {
    // Regression guard: `isBriefLoading` counts 'absent' as loading (it drives the
    // Prep tab spinner). Reusing it here would show the button on every meeting.
    invokeMock.mockResolvedValue(view({ briefStatus: 'absent' }));
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(result.current.hasPrep).toBe(false);
  });

  it('reports prep when a brief exists', async () => {
    invokeMock.mockResolvedValue(view({ briefStatus: 'ready', briefMarkdown: 'Last time…' }));
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(result.current.hasPrep).toBe(true));
  });

  it('reports prep while a brief is still generating', async () => {
    invokeMock.mockResolvedValue(view({ briefStatus: 'pending' }));
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(result.current.hasPrep).toBe(true));
  });

  it('reports prep for typed agenda notes alone', async () => {
    invokeMock.mockResolvedValue(view({ prepNotesMarkdown: '- Q3 roadmap' }));
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(result.current.hasPrep).toBe(true));
  });

  it('counts carried-over open items for the badge', async () => {
    invokeMock.mockResolvedValue(
      view({ openItems: [{ id: 'a' }, { id: 'b' }, { id: 'c' }] }),
    );
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(result.current.openItemCount).toBe(3));
    expect(result.current.hasPrep).toBe(true);
  });

  it('never calls the backend without a meeting id', async () => {
    const { result } = renderHook(() => usePrepAvailability(null));
    await waitFor(() => expect(result.current.hasPrep).toBe(false));
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('stays silent when the prep read fails — a recording must not surface prep errors', async () => {
    // Tauri's invoke rejects with a plain string, not an Error — matching that
    // here also sidesteps vitest attributing a constructed Error to the test.
    invokeMock.mockImplementationOnce(() => Promise.reject('db gone'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => usePrepAvailability('meeting-1'));
    await waitFor(() => expect(warn).toHaveBeenCalled());
    expect(result.current.hasPrep).toBe(false);
    warn.mockRestore();
  });
});
