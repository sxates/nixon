import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import { RecordingPostProcessingProvider } from '@/contexts/RecordingPostProcessingProvider';
import { DeferredBacklogProvider } from '@/contexts/DeferredBacklogProvider';

// spec 0051 WS2 finding 1 (review round 1) — useRecordingStop (mounted unconditionally
// inside RecordingPostProcessingProvider for the tray / global-shortcut stop path) now
// calls useBacklog(), which THROWS if no ancestor DeferredBacklogProvider is mounted
// (contexts/DeferredBacklogProvider.tsx:20-22). Before the layout.tsx fix,
// RecordingPostProcessingProvider was mounted OUTSIDE (and above) DeferredBacklogProvider,
// so this threw during render of the app's root layout on every route, and
// DeferredBacklogProvider was additionally absent entirely during onboarding. This test
// renders the REAL DeferredBacklogProvider (only its internal useDeferredBacklog hook is
// mocked — the provider component and its useContext plumbing are real) as the ANCESTOR
// of the real RecordingPostProcessingProvider, mirroring the corrected layout.tsx nesting.
// A regression that un-nests them again reproduces the exact crash here instead of only at
// app boot.

const { enqueueMeetingMock } = vi.hoisted(() => ({
  enqueueMeetingMock: vi.fn().mockResolvedValue({ accepted: true }),
}));

vi.mock('@/hooks/useDeferredBacklog', () => ({
  useDeferredBacklog: () => ({
    view: {
      items: [],
      pendingCount: 0,
      processing: false,
      active: null,
      activeOrdinal: 0,
      total: 0,
    },
    enqueueMeeting: enqueueMeetingMock,
    stop: vi.fn(),
    dismissDone: vi.fn(),
    startNow: vi.fn(),
  }),
}));

// useRecordingStop's own dependencies (same seam as hooks/__tests__/useRecordingStop.test.ts),
// mocked so the hook can mount without a real Tauri/DB backend. Deliberately NOT mocking
// '@/contexts/DeferredBacklogProvider' — that's the whole point of this test.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(() => () => {}),
  makeSafeUnlisten: vi.fn(() => () => {}),
}));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn(), replace: vi.fn() }) }));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({
    transcriptsRef: { current: [] },
    flushBuffer: vi.fn(),
    clearTranscripts: vi.fn(),
    meetingTitle: 'Test',
    markMeetingAsSaved: vi.fn(),
  }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    refetchMeetings: vi.fn(),
    setCurrentMeeting: vi.fn(),
    setMeetings: vi.fn(),
    meetings: [],
    setIsMeetingActive: vi.fn(),
    setActiveRecordingMeetingId: vi.fn(),
    currentMeeting: null,
    activeRecordingMeetingId: null,
  }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({
    status: 'idle',
    setStatus: vi.fn(),
    isStopping: false,
    isProcessing: false,
    isSaving: false,
  }),
  RecordingStatus: {
    IDLE: 'idle',
    STOPPING: 'stopping',
    PROCESSING_TRANSCRIPTS: 'processing',
    SAVING: 'saving',
    COMPLETED: 'completed',
    ERROR: 'error',
  },
}));
vi.mock('@/services/storageService', () => ({
  storageService: { saveMeeting: vi.fn(), getMeeting: vi.fn() },
}));
vi.mock('@/services/transcriptService', () => ({
  transcriptService: { getTranscriptionStatus: vi.fn() },
}));
vi.mock('@/lib/summary-language-preferences', () => ({
  applyPinnedSummaryLanguageToMeeting: vi.fn(),
  detectAndCacheSummaryLanguage: vi.fn(),
}));
vi.mock('sonner', () => ({ toast: { warning: vi.fn(), success: vi.fn(), error: vi.fn() } }));

describe('RecordingPostProcessingProvider — 0051 WS2 finding 1 (must mount inside DeferredBacklogProvider)', () => {
  it('does not throw "useBacklog must be used within a DeferredBacklogProvider" with the real layout.tsx nesting', () => {
    expect(() => {
      render(
        <DeferredBacklogProvider>
          <RecordingPostProcessingProvider>
            <div>child</div>
          </RecordingPostProcessingProvider>
        </DeferredBacklogProvider>,
      );
    }).not.toThrow();
  });

  it('throws when RecordingPostProcessingProvider is NOT nested inside DeferredBacklogProvider (documents the exact prior bug)', () => {
    // Silence the expected React error-boundary console noise for this one assertion.
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => {
      render(
        <RecordingPostProcessingProvider>
          <div>child</div>
        </RecordingPostProcessingProvider>,
      );
    }).toThrow('useBacklog must be used within a DeferredBacklogProvider');
    errorSpy.mockRestore();
  });
});
