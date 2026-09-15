import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';

/**
 * specs/0057 Plan 3, Task 5 — the channel strip is meeting IDENTITY, not transcript
 * chrome. It must render on the document itself, ABOVE the tabs, so "who is on this
 * reel" is visible no matter which tab is open (previously it was buried inside the
 * Transcript tab's control area and vanished on Summary / My notes).
 *
 * The assertion is structural and document-order based: the `role="table"` named
 * "Channels" must precede the `role="tablist"` in the DOM. The tab panels are stubbed —
 * this is about where the strip is MOUNTED, not what the panels render.
 */

const { invoke, router, searchParams, recordingState, sidebarState } = vi.hoisted(() => ({
  invoke: vi.fn(),
  router: { push: vi.fn(), replace: vi.fn(), back: vi.fn() },
  searchParams: new URLSearchParams('id=m-1'),
  recordingState: { isRecording: false },
  sidebarState: { activeRecordingMeetingId: null as string | null },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
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
// Panels are irrelevant to placement (and drag in BlockNote/Prep) — stub them out.
vi.mock('@/components/MeetingDetails/MeetingTabPanels', () => ({
  MeetingTabPanels: () => <div data-testid="tab-panels" />,
}));
vi.mock('@/components/Participants/ParticipantsPanel', () => ({
  ParticipantsPanel: () => <div data-testid="participants" />,
}));
// Page-level data hooks: the placement test cares about structure, not their behavior.
vi.mock('@/hooks/meeting-details/useMeetingData', () => ({
  useMeetingData: () => ({
    meetingTitle: 'Standup',
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

const meeting = {
  id: 'm-1',
  title: 'Standup',
  created_at: '2026-09-13T10:00:00Z',
  folder_path: '/rec/m-1',
  origin: 'recorded',
  reelNumber: 412,
};

/**
 * THREE speaker rows, but `spk_0` and `spk_1` are both assigned to the same Person —
 * `consolidateSpeakers` folds them into one channel, so the strip draws CH1-CH2 and the
 * identity line must agree (a raw `speakers.length` would claim 3 voices).
 */
const speakers = [
  { speakerKey: 'local', displayName: 'You', isLocal: true, segmentCount: 4 },
  {
    speakerKey: 'spk_0',
    displayName: 'Sarah Chen',
    isLocal: false,
    segmentCount: 3,
    personId: 'p-sarah',
    email: 'sarah@example.com',
  },
  {
    speakerKey: 'spk_1',
    displayName: 'Sarah Chen',
    isLocal: false,
    segmentCount: 2,
    personId: 'p-sarah',
    email: 'sarah@example.com',
  },
];

const transcriptRows = [
  { id: 't1', text: 'hi', speaker: 'local', audio_start_time: 0, audio_end_time: 30 },
  { id: 't2', text: 'hello', speaker: 'spk_0', audio_start_time: 30, audio_end_time: 50 },
];

beforeEach(() => {
  vi.clearAllMocks();
  recordingState.isRecording = false;
  sidebarState.activeRecordingMeetingId = null;
  invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_get_meeting_speakers':
        return speakers;
      case 'api_get_meeting_attendees':
        return { attendees: [], suggestion: null };
      case 'api_list_people_ranked':
        return [];
      case 'api_get_meeting_transcripts':
        return { transcripts: transcriptRows, total_count: transcriptRows.length, has_more: false };
      case 'api_get_meetings':
        return [{ id: 'm-1', durationSeconds: 50 }];
      default:
        return null;
    }
  });
});

describe('meeting-details channel strip placement (specs/0057 Plan 3, Task 5)', () => {
  it('renders the Channels strip ABOVE the tablist, on the document itself', async () => {
    render(<PageContent meeting={meeting} summaryData={null} />);

    const strip = await screen.findByRole('table', { name: 'Channels' });
    const tablist = screen.getByRole('tablist');

    // DOCUMENT_POSITION_FOLLOWING (4) — the tablist comes AFTER the strip.
    expect(
      strip.compareDocumentPosition(tablist) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('reports the CONSOLIDATED channel count as VOICES (not the raw speaker rows)', async () => {
    render(<PageContent meeting={meeting} summaryData={null} />);

    const strip = await screen.findByRole('table', { name: 'Channels' });
    // 3 speaker rows -> 2 channels (spk_0 + spk_1 share a personId): header + 2 rows.
    await waitFor(() => {
      expect(within(strip).getAllByRole('row')).toHaveLength(3);
    });
    // The reel-label card is gone (0.1.0 canvas feedback); the identity line carries the
    // voice count now, still consolidated.
    const line = screen.getByTestId('meeting-identity-line').textContent ?? '';
    expect(line).toContain('2 voices');
    expect(line).not.toContain('3 voices');
  });

  it('hides the strip while THIS meeting is the live recording', async () => {
    recordingState.isRecording = true;
    sidebarState.activeRecordingMeetingId = 'm-1';

    render(<PageContent meeting={meeting} summaryData={null} />);

    // The tablist proves the page rendered; the strip must not be there with it.
    await screen.findByRole('tablist');
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('api_get_meeting_speakers', expect.anything());
    });
    expect(screen.queryByRole('table', { name: 'Channels' })).toBeNull();
  });

  it('still shows the strip while a DIFFERENT meeting is recording', async () => {
    recordingState.isRecording = true;
    sidebarState.activeRecordingMeetingId = 'other-meeting';

    render(<PageContent meeting={meeting} summaryData={null} />);

    expect(await screen.findByRole('table', { name: 'Channels' })).toBeInTheDocument();
  });
});
