import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

/**
 * specs/0069b followup — "take me to the new meeting's details immediately" is a
 * CREATE-only behavior (Today's Add-meeting dialog). Editing an existing manual entry
 * from the meeting page itself (this dialog's `editing` mode) must never navigate —
 * there's nowhere to go, you're already on the page being edited. Mirrors
 * `manual-meeting-edit.test.tsx`'s setup; kept as a separate file so that suite's
 * existing assertions stay untouched.
 */

const { router, searchParams, recordingState, sidebarState } = vi.hoisted(() => ({
  router: { push: vi.fn(), replace: vi.fn(), back: vi.fn() },
  searchParams: new URLSearchParams('id=m-1'),
  recordingState: { isRecording: false },
  sidebarState: { activeRecordingMeetingId: null as string | null },
}));

const invoke = vi.fn(async (cmd: string) => {
  switch (cmd) {
    case 'api_get_meeting_speakers':
      return [];
    case 'api_get_meeting_attendees':
      return { attendees: [], suggestion: null };
    case 'api_list_people_ranked':
      return [];
    case 'api_get_meeting_transcripts':
      return { transcripts: [], total_count: 0, has_more: false };
    case 'api_get_meetings':
      return [];
    case 'api_update_manual_meeting':
      return undefined;
    default:
      return null;
  }
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('sonner', () => ({
  toast: { info: vi.fn(), success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));
vi.mock('next/navigation', () => ({
  useSearchParams: () => searchParams,
  useRouter: () => router,
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    refetchMeetings: vi.fn(),
    activeRecordingMeetingId: sidebarState.activeRecordingMeetingId,
  }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ modelConfig: {}, setModelConfig: vi.fn() }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: recordingState.isRecording }),
}));
vi.mock('@/components/MeetingDetails/MeetingTabPanels', () => ({
  MeetingTabPanels: () => <div data-testid="tab-panels" />,
}));
vi.mock('@/components/Participants/ParticipantsPanel', () => ({
  ParticipantsPanel: () => <div data-testid="participants" />,
}));
vi.mock('@/hooks/meeting-details/useMeetingData', () => ({
  useMeetingData: () => ({
    meetingTitle: 'Call with Sam',
    handleTitleChange: vi.fn(),
    handleSaveMeetingTitle: vi.fn(),
    updateMeetingTitle: vi.fn(),
    transcripts: [],
    aiSummary: null,
    setAiSummary: vi.fn(),
    blockNoteSummaryRef: { current: null },
    isEditingTitle: false,
    setIsEditingTitle: vi.fn(),
    isTitleDirty: false,
    isSaving: false,
    saveAllChanges: vi.fn(),
    handleSaveSummary: vi.fn(),
    handleSummaryChange: vi.fn(),
    setIsSummaryDirty: vi.fn(),
  }),
}));
vi.mock('@/hooks/meeting-details/useSummaryGeneration', () => ({
  useSummaryGeneration: () => ({}),
}));
vi.mock('@/hooks/meeting-details/useTemplates', () => ({
  useTemplates: () => ({ availableTemplates: [], selectedTemplate: null }),
}));
vi.mock('@/hooks/meeting-details/useCopyOperations', () => ({
  useCopyOperations: () => ({}),
}));
vi.mock('@/hooks/meeting-details/useMeetingOperations', () => ({
  useMeetingOperations: () => ({}),
}));
vi.mock('@/hooks/meeting-details/useAutoGenerateSummary', () => ({
  useAutoGenerateSummary: () => undefined,
}));
vi.mock('@/hooks/meeting-details/useModelSettings', () => ({
  useModelSettings: () => ({
    handleRegisterModalOpen: vi.fn(),
    handleOpenModelSettings: vi.fn(),
    handleSaveModelConfig: vi.fn(),
  }),
}));

import PageContent from '../page-content';

const manualMeeting = {
  id: 'm-manual',
  title: 'Call with Sam',
  created_at: '2026-09-20T15:00:00.000Z',
  origin: 'scheduled',
  isManualEntry: true,
  scheduledEndAt: '2026-09-20T15:30:00.000Z',
  joinUrl: 'https://zoom.us/j/1',
};

async function openOptionsMenu() {
  const trigger = await screen.findByRole('button', { name: 'Meeting options' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
}

beforeEach(() => {
  invoke.mockClear();
  router.push.mockClear();
  recordingState.isRecording = false;
  sidebarState.activeRecordingMeetingId = null;
});

describe('editing a manual meeting from its own page never navigates (specs/0069b followup)', () => {
  it('saves the edit and stays on the meeting page', async () => {
    render(<PageContent meeting={manualMeeting} summaryData={null} />);
    await openOptionsMenu();
    fireEvent.click(await screen.findByText('Edit date & time'));

    fireEvent.click(screen.getByRole('button', { name: /save/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        'api_update_manual_meeting',
        expect.objectContaining({ meetingId: 'm-manual' }),
      ),
    );
    expect(router.push).not.toHaveBeenCalled();
  });
});
