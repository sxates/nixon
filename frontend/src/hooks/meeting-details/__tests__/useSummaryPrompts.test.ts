import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

const { invokeMock, toastError } = vi.hoisted(() => ({ invokeMock: vi.fn(), toastError: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: toastError, info: vi.fn() },
}));

const sidebarState: { currentMeeting: { id: string; title: string } | null } = {
  currentMeeting: null,
};
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => sidebarState,
}));

import { useSummaryPrompts } from '@/hooks/meeting-details/useSummaryPrompts';
import type { MeetingPromptState, SummaryPrompt } from '@/lib/summary-prompts';

const REAL_ID = 'meeting-123e4567-e89b-42d3-a456-426614174000';

const prompt = (id: string): SummaryPrompt => ({
  id,
  name: id,
  body: 'b',
  sort_order: 0,
  is_default: false,
  extract_action_items: true,
  in_library: true,
  created_at: '',
  updated_at: '',
});

const baseState = (over: Partial<MeetingPromptState> = {}): MeetingPromptState => ({
  source: 'default',
  prompt_id: 'p1',
  prompt_name: 'p1',
  extract_action_items: true,
  custom_body: null,
  custom_extract_action_items: null,
  has_series: false,
  ...over,
});

let currentState: MeetingPromptState;

beforeEach(() => {
  vi.clearAllMocks();
  sidebarState.currentMeeting = null;
  currentState = baseState();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_list_summary_prompts':
        return Promise.resolve([prompt('p1'), prompt('p2')]);
      case 'api_get_meeting_prompt_state':
        return Promise.resolve(currentState);
      default:
        return Promise.resolve(null);
    }
  });
});

describe('useSummaryPrompts', () => {
  it('loads prompts and state for the explicit meeting id', async () => {
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state?.prompt_id).toBe('p1'));
    expect(result.current.prompts).toHaveLength(2);
    expect(invokeMock).toHaveBeenCalledWith('api_list_summary_prompts');
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_prompt_state', { meetingId: REAL_ID });
  });

  it('selectPrompt persists with applyToSeries = has_series, then refreshes', async () => {
    currentState = baseState({ has_series: true });
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state?.has_series).toBe(true));
    invokeMock.mockClear();
    currentState = baseState({ source: 'series', prompt_id: 'p2', has_series: true });
    await act(async () => {
      await result.current.selectPrompt('p2');
    });
    expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_summary_prompt', {
      meetingId: REAL_ID,
      promptId: 'p2',
      applyToSeries: true,
    });
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_prompt_state', { meetingId: REAL_ID });
    await waitFor(() => expect(result.current.state?.prompt_id).toBe('p2'));
  });

  it('queues a pick made with a null id and flushes when the id arrives', async () => {
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useSummaryPrompts(id),
      { initialProps: { id: null as string | null } },
    );
    await act(async () => {
      await result.current.selectPrompt('p2');
    });
    expect(invokeMock).not.toHaveBeenCalledWith('api_set_meeting_summary_prompt', expect.anything());
    rerender({ id: REAL_ID });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_summary_prompt', {
        meetingId: REAL_ID,
        promptId: 'p2',
        applyToSeries: false,
      }),
    );
  });

  it('does not let a late state load overwrite a pick made this mount', async () => {
    let resolveLoad: (v: MeetingPromptState) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_summary_prompts') return Promise.resolve([prompt('p1'), prompt('p2')]);
      if (cmd === 'api_get_meeting_prompt_state') {
        return new Promise((resolve) => {
          resolveLoad = resolve as (v: MeetingPromptState) => void;
        });
      }
      return Promise.resolve(null);
    });
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.prompts).toHaveLength(2));
    const initialResolve = resolveLoad;
    // Pick while the initial state load is still in flight.
    await act(async () => {
      void result.current.selectPrompt('p2');
    });
    // The stale initial load resolves with the old prompt.
    await act(async () => {
      initialResolve(baseState({ prompt_id: 'p1' }));
    });
    expect(result.current.state?.prompt_id).not.toBe('p1');
  });

  it('saveOneOff invokes api_set_meeting_custom_prompt', async () => {
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state).not.toBeNull());
    await act(async () => {
      await result.current.saveOneOff('x', false);
    });
    expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_custom_prompt', {
      meetingId: REAL_ID,
      body: 'x',
      extractActionItems: false,
    });
  });

  it('saveOneOff toasts the backend message and rethrows on failure', async () => {
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state).not.toBeNull());
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_set_meeting_custom_prompt'
        ? Promise.reject('The prompt is 4001 characters; the limit is 4000.')
        : Promise.resolve(currentState),
    );
    let caught: unknown;
    await act(async () => {
      try {
        await result.current.saveOneOff('x', true);
      } catch (e) {
        caught = e;
      }
    });
    expect(caught).toBeTruthy();
    expect(toastError).toHaveBeenCalledWith(
      expect.stringContaining('The prompt is 4001 characters'),
    );
  });

  it('clearOneOff sends body null', async () => {
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state).not.toBeNull());
    await act(async () => {
      await result.current.clearOneOff();
    });
    expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_custom_prompt', {
      meetingId: REAL_ID,
      body: null,
      extractActionItems: true,
    });
  });

  it('saveFollowup invokes api_save_custom_prompt_followup and returns the prompt', async () => {
    const saved = prompt('new');
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_save_custom_prompt_followup') return Promise.resolve(saved);
      if (cmd === 'api_list_summary_prompts') return Promise.resolve([prompt('p1')]);
      if (cmd === 'api_get_meeting_prompt_state') return Promise.resolve(currentState);
      return Promise.resolve(null);
    });
    const { result } = renderHook(() => useSummaryPrompts(REAL_ID));
    await waitFor(() => expect(result.current.state).not.toBeNull());
    let ret: SummaryPrompt | undefined;
    await act(async () => {
      ret = await result.current.saveFollowup({ name: 'N', toLibrary: true, toSeries: false });
    });
    expect(ret).toEqual(saved);
    expect(invokeMock).toHaveBeenCalledWith('api_save_custom_prompt_followup', {
      meetingId: REAL_ID,
      name: 'N',
      toLibrary: true,
      toSeries: false,
    });
  });

  it('never loads state against fabricated ids', async () => {
    sidebarState.currentMeeting = { id: 'intro-call', title: '+ New Call' };
    renderHook(() => useSummaryPrompts());
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_list_summary_prompts'));
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_meeting_prompt_state', expect.anything());
  });
});
