import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

/**
 * specs/0061 W4 (task 3) — page-content owns the toggle-to-clear contract for
 * `onSelectSpeaker`: selecting the already-selected key clears the filter;
 * otherwise it sets the filter, looks up the speaker's first segment
 * (api_first_segment_for_speaker), and hands the id to the deep-link machinery.
 * This locks that ownership piece specifically (ChannelStrip's click/Enter and
 * TranscriptPanel's filtered rendering are covered in their own unit tests) by
 * inspecting the `speakerFilter` MeetingTabPanels is actually handed — the same
 * structural-mock style channel-strip-placement.test.tsx uses.
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
// Stub the panels but surface the two props under test as data attributes, so
// this test can assert on page-content's OWN toggle logic without pulling in
// the full transcript/virtualization stack (covered separately).
vi.mock('@/components/MeetingDetails/MeetingTabPanels', () => ({
  MeetingTabPanels: ({
    speakerFilter,
    speakerFilterKeys,
    activeTab,
  }: {
    speakerFilter?: string | null;
    speakerFilterKeys?: string[] | null;
    activeTab?: string;
  }) => (
    <div
      data-testid="tab-panels"
      data-speaker-filter={speakerFilter ?? ''}
      data-speaker-filter-keys={(speakerFilterKeys ?? []).join(',')}
      data-active-tab={activeTab ?? ''}
    />
  ),
}));
vi.mock('@/components/Participants/ParticipantsPanel', () => ({
  ParticipantsPanel: () => <div data-testid="participants" />,
}));
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

const speakers = [
  { speakerKey: 'local', displayName: 'You', isLocal: true, segmentCount: 4 },
  { speakerKey: 'spk_0', displayName: 'Alex', isLocal: false, segmentCount: 3 },
];

// Ruling R37 — each row's own "Filter transcript to <name>" button (inside the
// channel cell) is the unambiguous way to select a speaker: the row itself stays
// `role="row"` (not `role="button"`), and the SpeakerChip's OWN nested rename
// `<button>` has just the bare name, not this fuller label, so there's no
// accessible-name collision.
async function findFilterButton(name: string): Promise<HTMLElement> {
  return screen.findByRole('button', { name: `Filter transcript to ${name}` });
}

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
        return { transcripts: [], total_count: 0, has_more: false };
      case 'api_get_meetings':
        return [{ id: 'm-1', durationSeconds: 50 }];
      case 'api_first_segment_for_speaker':
        return 'seg-99';
      default:
        return null;
    }
  });
});

describe('page-content — speaker select/clear toggle (specs/0061 W4 task 3)', () => {
  it('selecting a speaker sets the filter, switches to Transcript, and looks up their first line', async () => {
    render(<PageContent meeting={meeting} summaryData={null} requestSegmentScroll={vi.fn()} />);

    const filterButton = await findFilterButton('Alex');
    fireEvent.click(filterButton);

    await waitFor(() => {
      expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter', 'spk_0');
    });
    expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-active-tab', 'transcript');
    // ruling R36 — the backend command takes the group's full member-key list, not a
    // single `speakerKey`; for an ungrouped speaker that's just its own key.
    expect(invoke).toHaveBeenCalledWith(
      'api_first_segment_for_speaker',
      expect.objectContaining({ meetingId: 'm-1', speakerKeys: ['spk_0'] }),
    );
    expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter-keys', 'spk_0');
  });

  it('hands the resolved first-segment id to requestSegmentScroll', async () => {
    const requestSegmentScroll = vi.fn();
    render(
      <PageContent meeting={meeting} summaryData={null} requestSegmentScroll={requestSegmentScroll} />,
    );

    const filterButton = await findFilterButton('Alex');
    fireEvent.click(filterButton);

    await waitFor(() => {
      expect(requestSegmentScroll).toHaveBeenCalledWith('seg-99');
    });
  });

  it('clicking the same row again clears the filter', async () => {
    render(<PageContent meeting={meeting} summaryData={null} requestSegmentScroll={vi.fn()} />);

    const filterButton = await findFilterButton('Alex');
    fireEvent.click(filterButton);
    await waitFor(() => {
      expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter', 'spk_0');
    });

    fireEvent.click(filterButton);
    await waitFor(() => {
      expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter', '');
    });
  });

  it('does not hang or crash when the speaker has no first segment (null lookup)', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_meeting_speakers') return speakers;
      if (cmd === 'api_get_meeting_attendees') return { attendees: [], suggestion: null };
      if (cmd === 'api_list_people_ranked') return [];
      if (cmd === 'api_get_meeting_transcripts') return { transcripts: [], total_count: 0, has_more: false };
      if (cmd === 'api_get_meetings') return [{ id: 'm-1', durationSeconds: 50 }];
      if (cmd === 'api_first_segment_for_speaker') return null;
      return null;
    });
    const requestSegmentScroll = vi.fn();
    render(
      <PageContent meeting={meeting} summaryData={null} requestSegmentScroll={requestSegmentScroll} />,
    );

    const filterButton = await findFilterButton('Alex');
    fireEvent.click(filterButton);

    await waitFor(() => {
      expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter', 'spk_0');
    });
    expect(requestSegmentScroll).not.toHaveBeenCalled();
  });

  it('looks up EVERY member key of a consolidated group, not just the primary (ruling R36)', async () => {
    // spk_0 and spk_0b are consolidated into one channel (same personId) — clicking
    // the row must resolve the group's true earliest line across BOTH raw keys.
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_meeting_speakers') {
        return [
          { speakerKey: 'local', displayName: 'You', isLocal: true },
          { speakerKey: 'spk_0', displayName: 'Alex', isLocal: false, personId: 'p-alex' },
          { speakerKey: 'spk_0b', displayName: 'Alex', isLocal: false, personId: 'p-alex' },
        ];
      }
      if (cmd === 'api_get_meeting_attendees') return { attendees: [], suggestion: null };
      if (cmd === 'api_list_people_ranked') return [];
      if (cmd === 'api_get_meeting_transcripts') return { transcripts: [], total_count: 0, has_more: false };
      if (cmd === 'api_get_meetings') return [{ id: 'm-1', durationSeconds: 50 }];
      if (cmd === 'api_first_segment_for_speaker') return 'seg-99';
      return null;
    });

    render(<PageContent meeting={meeting} summaryData={null} requestSegmentScroll={vi.fn()} />);

    const filterButton = await findFilterButton('Alex');
    fireEvent.click(filterButton);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        'api_first_segment_for_speaker',
        expect.objectContaining({ meetingId: 'm-1', speakerKeys: ['spk_0', 'spk_0b'] }),
      );
    });
    expect(screen.getByTestId('tab-panels')).toHaveAttribute(
      'data-speaker-filter-keys',
      'spk_0,spk_0b',
    );
  });

  it('drops a stale first-segment resolution when a second click lands before it resolves (ruling R39)', async () => {
    // Two speakers, each with a controllable (not-yet-resolved) lookup promise.
    let resolveAlex!: (id: string | null) => void;
    let resolveJordan!: (id: string | null) => void;
    const alexPromise = new Promise<string | null>((res) => {
      resolveAlex = res;
    });
    const jordanPromise = new Promise<string | null>((res) => {
      resolveJordan = res;
    });
    invoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'api_get_meeting_speakers') {
        return [
          { speakerKey: 'local', displayName: 'You', isLocal: true },
          { speakerKey: 'spk_0', displayName: 'Alex', isLocal: false },
          { speakerKey: 'spk_1', displayName: 'Jordan', isLocal: false },
        ];
      }
      if (cmd === 'api_get_meeting_attendees') return { attendees: [], suggestion: null };
      if (cmd === 'api_list_people_ranked') return [];
      if (cmd === 'api_get_meeting_transcripts') return { transcripts: [], total_count: 0, has_more: false };
      if (cmd === 'api_get_meetings') return [{ id: 'm-1', durationSeconds: 50 }];
      if (cmd === 'api_first_segment_for_speaker') {
        const keys = (args?.speakerKeys as string[] | undefined) ?? [];
        return keys.includes('spk_0') ? alexPromise : jordanPromise;
      }
      return null;
    });

    const requestSegmentScroll = vi.fn();
    render(
      <PageContent meeting={meeting} summaryData={null} requestSegmentScroll={requestSegmentScroll} />,
    );

    const alexButton = await findFilterButton('Alex');
    fireEvent.click(alexButton); // kicks off Alex's (still-pending) lookup

    const jordanButton = await findFilterButton('Jordan');
    fireEvent.click(jordanButton); // switches the filter before Alex's lookup resolves

    await waitFor(() => {
      expect(screen.getByTestId('tab-panels')).toHaveAttribute('data-speaker-filter', 'spk_1');
    });

    // Alex's lookup finally resolves AFTER Jordan is already selected — must be dropped.
    resolveAlex('seg-alex-first-line');
    await Promise.resolve();
    await Promise.resolve();
    expect(requestSegmentScroll).not.toHaveBeenCalledWith('seg-alex-first-line');

    // Jordan's own (current) lookup still works normally.
    resolveJordan('seg-jordan-first-line');
    await waitFor(() => {
      expect(requestSegmentScroll).toHaveBeenCalledWith('seg-jordan-first-line');
    });
    expect(requestSegmentScroll).toHaveBeenCalledTimes(1);
  });
});
