import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

const { sidebar, transcripts, invokeMock, toastErrorMock } = vi.hoisted(() => ({
  sidebar: {
    activeRecordingMeetingId: 'm1' as string | null,
    refetchMeetings: vi.fn(),
    setCurrentMeeting: vi.fn(),
  },
  transcripts: { meetingTitle: 'Old name', setMeetingTitle: vi.fn() },
  invokeMock: vi.fn(),
  toastErrorMock: vi.fn(),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => sidebar }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => transcripts }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({ toast: { error: toastErrorMock, success: vi.fn() } }));

import { useRecordingTitleEdit } from '@/hooks/useRecordingTitleEdit';

beforeEach(() => {
  Object.assign(sidebar, { activeRecordingMeetingId: 'm1' });
  Object.assign(transcripts, { meetingTitle: 'Old name' });
  vi.clearAllMocks();
  invokeMock.mockResolvedValue(undefined);
});

describe('useRecordingTitleEdit', () => {
  it('a committed rename reaches TranscriptContext, the DB, and the sidebar', async () => {
    const { result } = renderHook(() => useRecordingTitleEdit());
    act(() => result.current.startEditingTitle());
    act(() => result.current.setTitleDraft('  New name  '));
    await act(async () => { result.current.commitTitleEdit(); });

    expect(transcripts.setMeetingTitle).toHaveBeenCalledWith('New name');
    expect(invokeMock).toHaveBeenCalledWith('api_save_meeting_title', { meetingId: 'm1', title: 'New name' });
    expect(sidebar.setCurrentMeeting).toHaveBeenCalledWith({ id: 'm1', title: 'New name' });
  });

  it('a no-op edit touches nothing', async () => {
    const { result } = renderHook(() => useRecordingTitleEdit());
    act(() => result.current.startEditingTitle());
    act(() => result.current.setTitleDraft('Old name'));
    await act(async () => { result.current.commitTitleEdit(); });

    expect(transcripts.setMeetingTitle).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
    expect(sidebar.setCurrentMeeting).not.toHaveBeenCalled();
  });

  it('does not touch the sidebar when there is no authoritative meeting id yet', async () => {
    Object.assign(sidebar, { activeRecordingMeetingId: null });
    const { result } = renderHook(() => useRecordingTitleEdit());
    act(() => result.current.startEditingTitle());
    act(() => result.current.setTitleDraft('New name'));
    await act(async () => { result.current.commitTitleEdit(); });

    expect(invokeMock).not.toHaveBeenCalled();
    expect(sidebar.setCurrentMeeting).not.toHaveBeenCalled();
  });
});
