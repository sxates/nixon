import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import type { WrapUpState } from '@/lib/wrap-up';

// specs/0083 W2b — when Rust reports a meeting saved, the webview tidies up and, if anyone
// can see it, says so. It never navigates: the owner was pulled out of another meeting or
// prep every time a recording ended (2026-11-05), and `/record` stays put too.

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  push: vi.fn(),
  replace: vi.fn(),
  pathname: '/',
  toast: {
    success: vi.fn(),
    warning: vi.fn(),
    info: vi.fn(),
    error: vi.fn(),
  },
  transcripts: { markMeetingAsSaved: vi.fn(), clearTranscripts: vi.fn() },
  sidebar: {
    refetchMeetings: vi.fn(),
    setActiveRecordingMeetingId: vi.fn(),
    setIsMeetingActive: vi.fn(),
    currentMeeting: null as { id: string; title: string } | null,
    setCurrentMeeting: vi.fn(),
  },
}));

vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, handler: (event: { payload: unknown }) => void) => {
    h.listeners.set(event, handler);
    return () => h.listeners.delete(event);
  },
}));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: h.push, replace: h.replace }),
  usePathname: () => h.pathname,
}));
vi.mock('sonner', () => ({ toast: h.toast }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => h.transcripts }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => h.sidebar }));

import { useWrapUpEffects } from '@/hooks/useWrapUpEffects';

function state(stage: WrapUpState['stage'], extra: Partial<WrapUpState> = {}): WrapUpState {
  return {
    id: 7,
    meetingId: 'm-1',
    source: 'user',
    stage,
    title: 'Pricing sync',
    transcriptionIncomplete: false,
    reason: null,
    message: null,
    toast: null,
    failedStage: null,
    ...extra,
  };
}

async function emit(s: WrapUpState) {
  await act(async () => {
    h.listeners.get('meeting-wrap-up')!({ payload: s });
    await Promise.resolve();
  });
}

/** Put the window on `url` (jsdom keeps it in `window.location`) and the hook's pathname. */
function at(url: string) {
  window.history.replaceState(null, '', url);
  h.pathname = url.split('?')[0];
}

function setVisibility(value: 'visible' | 'hidden') {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => value });
}

beforeEach(() => {
  vi.clearAllMocks();
  h.listeners.clear();
  h.sidebar.currentMeeting = null;
  h.transcripts.markMeetingAsSaved.mockResolvedValue(undefined);
  h.sidebar.refetchMeetings.mockResolvedValue(undefined);
  setVisibility('visible');
  at('/');
});
afterEach(() => setVisibility('visible'));

describe('on saved, nothing navigates (specs/0083 owner decision)', () => {
  it.each([
    '/record',
    '/prep?id=m-9',
    '/meeting-details?id=m-other',
    '/meeting-details?id=m-1',
    '/settings?tab=recording',
    '/',
  ])('stays on %s through every stage', async (url) => {
    at(url);
    renderHook(() => useWrapUpEffects());
    for (const stage of ['stopping', 'draining', 'saving', 'saved', 'diarizing', 'summarizing', 'done'] as const) {
      await emit(state(stage));
    }
    expect(h.push).not.toHaveBeenCalled();
    expect(h.replace).not.toHaveBeenCalled();
  });
});

describe('the saved toast', () => {
  it('shows "Meeting saved — processing" with a View that opens the meeting', async () => {
    at('/prep?id=m-9');
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));

    expect(h.toast.success).toHaveBeenCalledTimes(1);
    const [title, options] = h.toast.success.mock.calls[0];
    expect(title).toBe('Meeting saved — processing');
    expect(options.action.label).toBe('View');
    options.action.onClick();
    expect(h.push).toHaveBeenCalledWith('/meeting-details?id=m-1');
  });

  it('shows on /record, which stays put', async () => {
    at('/record');
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.toast.success).toHaveBeenCalledTimes(1);
    expect(h.push).not.toHaveBeenCalled();
  });

  it('does not show while the window is hidden', async () => {
    setVisibility('hidden');
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.toast.success).not.toHaveBeenCalled();
  });

  it('does not show when the window already shows that meeting', async () => {
    at('/meeting-details?id=m-1');
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.toast.success).not.toHaveBeenCalled();
  });

  it('shows only for saved, not for the stages after it', async () => {
    renderHook(() => useWrapUpEffects());
    for (const stage of ['saved', 'diarizing', 'summarizing', 'done'] as const) {
      await emit(state(stage));
    }
    expect(h.toast.success).toHaveBeenCalledTimes(1);
  });
});

describe('tidying up after saved', () => {
  it('marks the recovery copy saved, refetches, and clears the live recording id', async () => {
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.transcripts.markMeetingAsSaved).toHaveBeenCalledTimes(1);
    expect(h.sidebar.refetchMeetings).toHaveBeenCalledTimes(1);
    expect(h.sidebar.setActiveRecordingMeetingId).toHaveBeenCalledWith(null);
    expect(h.sidebar.setIsMeetingActive).toHaveBeenCalledWith(false);
  });

  it('clears the live transcript at once off the recorder', async () => {
    at('/prep');
    renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.transcripts.clearTranscripts).toHaveBeenCalledTimes(1);
  });

  it('on /record, keeps the transcript until you leave', async () => {
    at('/record');
    const { rerender } = renderHook(() => useWrapUpEffects());
    await emit(state('saved'));
    expect(h.transcripts.clearTranscripts).not.toHaveBeenCalled();

    at('/');
    rerender();
    expect(h.transcripts.clearTranscripts).toHaveBeenCalledTimes(1);
  });

  it('shows Rust’s "Transcription didn’t fully finish" warning once', async () => {
    renderHook(() => useWrapUpEffects());
    const warning = { title: 'Transcription didn’t fully finish', description: 'We saved…' };
    await emit(state('saved', { transcriptionIncomplete: true, toast: warning }));
    await emit(state('diarizing'));
    expect(h.toast.warning).toHaveBeenCalledTimes(1);
    expect(h.toast.warning).toHaveBeenCalledWith(warning.title, expect.objectContaining({ description: warning.description }));
  });

  it('says so when the wrap-up failed', async () => {
    renderHook(() => useWrapUpEffects());
    await emit(state('failed', { failedStage: 'saving', message: 'The database isn’t ready.' }));
    expect(h.toast.error).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ description: 'The database isn’t ready.' }),
    );
    expect(h.push).not.toHaveBeenCalled();
  });
});
