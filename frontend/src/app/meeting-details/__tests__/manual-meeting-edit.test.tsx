import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

/**
 * specs/0069b review fix 2 — the owner's real-app smoke test: "There's no way to edit
 * the meeting date/time after I create it, my only option is to delete." Edit belongs
 * on the meeting page's "…" options menu (the Today timeline's hover `⋯` is untouched,
 * house idiom shared with All-meetings). Offered ONLY for a manual entry that hasn't
 * been recorded yet — `meeting.isManualEntry && meeting.origin === 'scheduled'` — which
 * mirrors the backend's own gate (`update_manual_scheduled` requires `origin = 'scheduled'
 * AND calendar_event_id LIKE 'nixon-manual:%'`). `isManualEntry` is backend-computed so
 * no `nixon-manual:` prefix literal lives in the frontend.
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

const recordedMeeting = {
  id: 'm-recorded',
  title: 'Standup',
  created_at: '2026-09-13T10:00:00Z',
  folder_path: '/rec/m-recorded',
  origin: 'recorded',
  isManualEntry: false,
};

// A calendar-backed scheduled row: NOT manual, even though origin is also 'scheduled'.
const calendarScheduledMeeting = {
  id: 'm-cal',
  title: 'Board sync',
  created_at: '2026-09-21T09:00:00Z',
  origin: 'scheduled',
  isManualEntry: false,
  calendarEventId: undefined, // omitted so ScheduledRecordControl doesn't mount here
};

async function openOptionsMenu() {
  const trigger = await screen.findByRole('button', { name: 'Meeting options' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
}

beforeEach(() => {
  invoke.mockClear();
  recordingState.isRecording = false;
  sidebarState.activeRecordingMeetingId = null;
});

describe('meeting page Edit affordance for manual entries (specs/0069b review fix 2)', () => {
  it('offers "Edit date & time" for a manual scheduled meeting', async () => {
    render(<PageContent meeting={manualMeeting} summaryData={null} />);
    await openOptionsMenu();
    expect(await screen.findByText('Edit date & time')).toBeInTheDocument();
  });

  it('does not offer Edit for a recorded meeting', async () => {
    render(<PageContent meeting={recordedMeeting} summaryData={null} />);
    await openOptionsMenu();
    // The menu did open (Delete is always there) — Edit specifically is absent.
    await screen.findByText('Delete meeting');
    expect(screen.queryByText('Edit date & time')).toBeNull();
  });

  it('does not offer Edit for a calendar-backed scheduled meeting (not manual)', async () => {
    render(<PageContent meeting={calendarScheduledMeeting} summaryData={null} />);
    await openOptionsMenu();
    await screen.findByText('Delete meeting');
    expect(screen.queryByText('Edit date & time')).toBeNull();
  });

  it('editing the time calls api_update_manual_meeting and refreshes the page data', async () => {
    const onRefetchTranscripts = vi.fn().mockResolvedValue(undefined);
    const onMeetingUpdated = vi.fn().mockResolvedValue(undefined);
    render(
      <PageContent
        meeting={manualMeeting}
        summaryData={null}
        onRefetchTranscripts={onRefetchTranscripts}
        onMeetingUpdated={onMeetingUpdated}
      />,
    );
    await openOptionsMenu();
    fireEvent.click(await screen.findByText('Edit date & time'));

    // Seeded from the manual entry's occurrence start/end (created_at / scheduledEndAt),
    // NOT a creation timestamp.
    expect(await screen.findByLabelText(/title/i)).toHaveValue('Call with Sam');

    fireEvent.click(screen.getByRole('button', { name: /save/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        'api_update_manual_meeting',
        expect.objectContaining({ meetingId: 'm-manual', title: 'Call with Sam' }),
      ),
    );
    await waitFor(() => expect(onRefetchTranscripts).toHaveBeenCalled());
    await waitFor(() => expect(onMeetingUpdated).toHaveBeenCalled());
  });
});
