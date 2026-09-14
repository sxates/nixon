import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useEffect, useState } from 'react';
import { render, waitFor } from '@testing-library/react';
import type { BacklogItem, BacklogItemStatus } from '@/lib/deferred-backlog';

/**
 * spec 0051 final review, Finding 1 — the SUCCESS path must not tell the user that
 * processing did NOT happen.
 *
 * On a 'process-now' stop, `useRecordingStop` writes `processing_mode='defer'` and hands
 * the meeting to the deferred backlog, which clears the marker only at the END of its
 * multi-minute retranscribe → diarize → summarize pipeline. Two seconds later the stop
 * path navigates here (`?source=recording`). Reading the marker alone, the auto-summary
 * gate classified that as 'deferred' and toasted "This meeting still needs processing.
 * Use 'Process now'..." — while the backlog pill was visibly processing it.
 *
 * These tests exercise the PAGE, not just the pure gate: they assert that the page reads
 * the shared backlog (`useBacklog`) and threads it into the gate. A page that computes
 * the gate from the marker alone fails them.
 */

const {
  invoke, toastInfo, paginated, loadingPaginated, sidebar, router, searchParams, deepLink,
} = vi.hoisted(() => {
  // Stable identities: the page syncs metadata+transcripts into state in an effect, so a
  // fresh object per render would loop ("Maximum update depth exceeded") in jsdom.
  const segments = [{ id: 's1', text: 'hello' }];
  return {
    invoke: vi.fn(),
    toastInfo: vi.fn(),
    paginated: {
      metadata: {
        id: 'm-1',
        title: 'Standup',
        created_at: '2026-08-14T10:00:00Z',
        updated_at: '2026-08-14T10:30:00Z',
        folder_path: '/rec/m-1',
      },
      segments,
      transcripts: segments,
      isLoading: false,
      isLoadingMore: false,
      hasMore: false,
      totalCount: 1,
      loadedCount: 1,
      loadMore: vi.fn(),
      refetch: vi.fn(),
      error: null,
    },
    sidebar: {
      setCurrentMeeting: vi.fn(),
      refetchMeetings: vi.fn(),
      stopSummaryPolling: vi.fn(),
      isMeetingActive: false,
      activeRecordingMeetingId: null,
    },
    router: { push: vi.fn(), replace: vi.fn() },
    searchParams: new URLSearchParams('id=m-1&source=recording'),
    deepLink: { pendingSegmentId: null, consume: vi.fn() },
    loadingPaginated: {
      metadata: null,
      segments: [],
      transcripts: [],
      isLoading: true,
      isLoadingMore: false,
      hasMore: false,
      totalCount: 0,
      loadedCount: 0,
      loadMore: vi.fn(),
      refetch: vi.fn(),
      error: null,
    },
  };
});

let backlogItems: BacklogItem[] = [];

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('sonner', () => ({
  toast: { info: toastInfo, success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));
vi.mock('next/navigation', () => ({
  useSearchParams: () => searchParams,
  useRouter: () => router,
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({
    view: {
      items: backlogItems,
      pendingCount: backlogItems.length,
      processing: backlogItems.length > 0,
      active: backlogItems[0] ?? null,
      activeOrdinal: backlogItems.length ? 1 : 0,
      total: backlogItems.length,
    },
    enqueueMeeting: vi.fn(),
    stop: vi.fn(),
    dismissDone: vi.fn(),
    startNow: vi.fn(),
  }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => sidebar,
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ isAutoSummary: true }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
// Mirrors the real hook's timing: metadata/transcripts land a render AFTER mount, which
// is what lets the page's mount-time reset effects run before the metadata sync effect.
vi.mock('@/hooks/usePaginatedTranscripts', () => ({
  usePaginatedTranscripts: () => {
    const [loaded, setLoaded] = useState(false);
    useEffect(() => {
      setLoaded(true);
    }, []);
    return loaded ? paginated : loadingPaginated;
  },
}));
vi.mock('@/hooks/useSegmentDeepLink', () => ({
  useSegmentDeepLink: () => deepLink,
}));
vi.mock('../page-content', () => ({
  default: () => <div data-testid="page-content" />,
}));

import MeetingDetails from '../page';

const item = (id: string, status: BacklogItemStatus): BacklogItem => ({
  meeting: { id, title: id, folderPath: `/rec/${id}`, transcriptCount: 3 },
  status,
});

beforeEach(() => {
  vi.clearAllMocks();
  backlogItems = [];
  invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_get_summary':
        return { status: 'idle', data: null };
      case 'api_get_meeting_processing_mode':
        return 'defer'; // written by the 'process-now' stop path BEFORE the handoff
      case 'api_meeting_audio_available':
        return true;
      default:
        return null;
    }
  });
});

describe('meeting-details auto-summary skip toast (0051 Finding 1)', () => {
  it('stays SILENT when the backlog is already processing this defer-marked meeting', async () => {
    backlogItems = [item('m-1', 'transcribing')];

    render(<MeetingDetails />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('api_get_meeting_processing_mode', {
        meetingId: 'm-1',
      });
    });
    // Let any queued toast flush before asserting the negative.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(toastInfo).not.toHaveBeenCalled();
  });

  it('still toasts "Processing deferred" when the meeting is genuinely waiting', async () => {
    backlogItems = []; // nothing in flight — the handoff never landed, or it's battery-deferred

    render(<MeetingDetails />);

    await waitFor(() => {
      expect(toastInfo).toHaveBeenCalledWith(
        'Processing deferred',
        expect.objectContaining({
          description: expect.stringContaining('Process now'),
        }),
      );
    });
  });
});
