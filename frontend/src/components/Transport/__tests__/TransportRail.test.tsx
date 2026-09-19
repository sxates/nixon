import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';

const { state, sidebar, backlog, llm, pauseMock, resumeMock, stopMock, toastErrorMock, transcripts, invokeMock } = vi.hoisted(() => ({
  state: { isRecording: false, isPaused: false, isActive: false, status: 'idle', activeDuration: null as number | null, recordingDuration: null as number | null, isStopping: false, isProcessing: false, isSaving: false },
  sidebar: { isCollapsed: true, handleRecordingToggle: vi.fn(), activeRecordingMeetingId: null as string | null, currentMeeting: null as { id: string; title: string } | null },
  backlog: { view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 }, stop: vi.fn(), startNow: vi.fn(), dismissDone: vi.fn(), enqueueMeeting: vi.fn() },
  // The provider's context value is FLAT (`{...view, dismiss}` — LlmActivityProvider.tsx),
  // so the fixture mirrors that shape rather than nesting a `view`.
  llm: { running: [] as unknown[], history: [] as unknown[], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() },
  pauseMock: vi.fn(), resumeMock: vi.fn(), stopMock: vi.fn(), toastErrorMock: vi.fn(),
  transcripts: { meetingTitle: 'Pricing sync' },
  invokeMock: vi.fn(),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => state }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => sidebar }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useOptionalLlmActivity: () => llm }));
// A real (unmocked) useState, not a fixture: the popover's open/close is local UI state,
// not something any test needs to seed or assert on directly — only the resulting DOM.
vi.mock('@/contexts/QueueOpenContext', async () => {
  const react = await import('react');
  return { useQueueOpen: () => { const [open, setOpen] = react.useState(false); return { open, setOpen }; } };
});
vi.mock('@/services/recordingService', () => ({ recordingService: { pauseRecording: pauseMock, resumeRecording: resumeMock } }));
vi.mock('@/lib/recording-stop', () => ({ requestFullRecordingStop: stopMock }));
vi.mock('sonner', () => ({ toast: { error: toastErrorMock, success: vi.fn() } }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => transcripts }));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => ({ rms: 0.4, peak: 0.5, peakLatched: false }) }));
vi.mock('@/hooks/useMicGate', () => ({ useMicGate: () => false }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }), usePathname: () => '/' }));

import { TransportRail } from '@/components/Transport/TransportRail';

beforeEach(() => {
  Object.assign(state, { isRecording: false, isPaused: false, isActive: false, status: 'idle', activeDuration: null, recordingDuration: null, isStopping: false, isProcessing: false, isSaving: false });
  Object.assign(llm, { running: [], history: [], hasFailure: false });
  Object.assign(sidebar, { isCollapsed: true, activeRecordingMeetingId: null, currentMeeting: null });
  // The queue fixtures below mutate `backlog.view` in place — without this reset the last
  // test to set it leaks its items into every test that runs after (fixture leak, review).
  backlog.view = { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 };
  Object.assign(transcripts, { meetingTitle: 'Pricing sync' });
  vi.clearAllMocks();
  pauseMock.mockResolvedValue(undefined);
  resumeMock.mockResolvedValue(undefined);
  invokeMock.mockResolvedValue(undefined);
});

// specs/0057 §3.1 state table + decision 7: one rail, one state vocabulary.
describe('TransportRail', () => {
  it('idle: Deck ready, REC starts a recording, HOLD/STOP disabled', () => {
    render(<TransportRail />);
    expect(screen.getByRole('region', { name: 'Transport' })).toBeTruthy();
    expect(screen.getByText(/deck ready/i)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Start recording' }));
    expect(sidebar.handleRecordingToggle).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Pause recording' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeDisabled();
  });
  it('recording: REC lit, counter runs, HOLD pauses, STOP requests the full stop', () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 252 });
    render(<TransportRail />);
    expect(screen.getByRole('region', { name: 'Recording in progress' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Recording' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByRole('button', { name: 'Recording' })).toBeDisabled();
    expect(screen.getByRole('timer').textContent?.replace(/\s/g, '')).toBe('00:04:12');
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));
    expect(pauseMock).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Stop recording' }));
    expect(stopMock).toHaveBeenCalled();
  });
  it('paused: HOLD lit and labelled Resume, counter amber', () => {
    Object.assign(state, { isRecording: true, isPaused: true, isActive: true, status: 'recording', activeDuration: 10 });
    render(<TransportRail />);
    const hold = screen.getByRole('button', { name: 'Resume recording' });
    expect(hold.getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByRole('timer').dataset.tone).toBe('amber');
    fireEvent.click(hold);
    expect(resumeMock).toHaveBeenCalled();
  });
  // RecordingStatus.STARTING: the tap is arming and `isRecording` is still false — the rail
  // must not fall back to "Deck ready" and offer REC a second time (review round 1).
  it('starting: armed, REC lit, HOLD/STOP disabled', () => {
    Object.assign(state, { isRecording: false, status: 'starting' });
    render(<TransportRail />);
    expect(screen.getByRole('region', { name: 'Recording in progress' })).toBeTruthy();
    expect(screen.getByText(/starting/i)).toBeTruthy();
    // REC stays LIT (the deck is armed) but is now visibly/a11y disabled: it was already a
    // no-op outside idle, so the key must say so rather than look pressable.
    expect(screen.getByRole('button', { name: 'Recording' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByRole('button', { name: 'Recording' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Pause recording' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeDisabled();
  });
  it('finalizing: keys disabled and Saving shown', () => {
    Object.assign(state, { isRecording: false, status: 'saving', isSaving: true, recordingDuration: 900 });
    render(<TransportRail />);
    expect(screen.getByText(/saving/i)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeDisabled();
  });
  // A failed pause used to be swallowed by `void` — HOLD looked like it worked while the
  // deck kept rolling (retired GlobalRecordingBar's try/catch + toast, restored here).
  it('HOLD surfaces a failed pause as a toast', async () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 5 });
    pauseMock.mockRejectedValue(new Error('device gone'));
    render(<TransportRail />);
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));
    await waitFor(() =>
      expect(toastErrorMock).toHaveBeenCalledWith('Could not pause recording', {
        description: 'Please try again, or open the recording to use the full controls.',
      }),
    );
  });
  it('HOLD ignores a second click while the first is in flight', async () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 5 });
    let release: () => void = () => {};
    pauseMock.mockImplementation(() => new Promise<void>((r) => { release = r; }));
    render(<TransportRail />);
    const hold = screen.getByRole('button', { name: 'Pause recording' });
    fireEvent.click(hold);
    await waitFor(() => expect(hold).toBeDisabled());
    fireEvent.click(hold);
    expect(pauseMock).toHaveBeenCalledTimes(1);
    release();
    await waitFor(() => expect(hold).not.toBeDisabled());
  });
  // Plan 2 re-review: a pause/resume invoke that never settles must not wedge HOLD for the
  // rest of the session — any phase transition releases the in-flight guard.
  it('HOLD un-wedges when the phase changes under a never-settling invoke', async () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 5 });
    pauseMock.mockImplementation(() => new Promise<void>(() => {}));
    const { rerender } = render(<TransportRail />);
    const hold = screen.getByRole('button', { name: 'Pause recording' });
    fireEvent.click(hold);
    await waitFor(() => expect(hold).toBeDisabled());

    // The backend never answers, but the recording did pause — the phase moved.
    Object.assign(state, { isPaused: true });
    rerender(<TransportRail />);
    const resume = screen.getByRole('button', { name: 'Resume recording' });
    await waitFor(() => expect(resume).not.toBeDisabled());

    // And the guard really is clear: the next press reaches the service.
    fireEvent.click(resume);
    expect(resumeMock).toHaveBeenCalledTimes(1);
  });
  // Review round 1: `isPaused` flips on a backend event, which can beat the pause command's own
  // promise — so the FIRST press's `finally` must not clear the busy state of a SECOND press
  // started after the phase moved on (the generation counter, not a bare boolean).
  it('a superseded HOLD press does not clear the next press in flight', async () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 5 });
    let releasePause: () => void = () => {};
    pauseMock.mockImplementation(() => new Promise<void>((r) => { releasePause = r; }));
    resumeMock.mockImplementation(() => new Promise<void>(() => {}));
    const { rerender } = render(<TransportRail />);
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));

    // The backend event lands before the pause promise settles.
    Object.assign(state, { isPaused: true });
    rerender(<TransportRail />);
    const resume = screen.getByRole('button', { name: 'Resume recording' });
    await waitFor(() => expect(resume).not.toBeDisabled());
    fireEvent.click(resume);
    await waitFor(() => expect(resume).toBeDisabled());

    // Now the stale pause finally runs — it must NOT re-enable the key under the live resume.
    releasePause();
    await act(async () => { await Promise.resolve(); });
    expect(resume).toBeDisabled();
  });
  // Off the recorded meeting's own page `currentMeeting` is someone else — the live session's
  // title (TranscriptContext) beats the bare "Recording" literal.
  it('shows the live meeting title when viewing a different meeting', () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 5 });
    Object.assign(sidebar, { activeRecordingMeetingId: 'live-1', currentMeeting: { id: 'other-2', title: 'Old retro' } });
    render(<TransportRail />);
    expect(screen.getByText('Pricing sync')).toBeTruthy();
    expect(screen.queryByText('Old retro')).toBeNull();
  });
  // STOP is a momentary key, never a toggle — it must not carry aria-pressed (controller ruling 2).
  it('STOP carries no aria-pressed state', () => {
    render(<TransportRail />);
    expect(screen.getByRole('button', { name: 'Stop recording' }).hasAttribute('aria-pressed')).toBe(false);
  });
  // The lamp inside the labelled trigger must not pollute its accessible name.
  it('queue trigger is named by its own text, not the lamp', () => {
    render(<TransportRail />);
    expect(screen.getByRole('button', { name: 'Queue 0 Idle' })).toBeTruthy();
  });
  it('queue: shows count and lamp, opens the panel', () => {
    backlog.view = { items: [{ meeting: { id: 'a', title: 'Hiring loop debrief', folderPath: '/x', transcriptCount: 1 }, status: 'transcribing' }], pendingCount: 0, processing: true, active: null, activeOrdinal: 1, total: 1 } as typeof backlog.view;
    render(<TransportRail />);
    const btn = screen.getByRole('button', { name: /queue/i });
    expect(btn.textContent).toMatch(/1/);
    expect(btn.textContent).toMatch(/transcribing/i);
    fireEvent.click(btn);
    expect(screen.getByRole('dialog', { name: /queue/i })).toBeTruthy();
    expect(screen.getByText('Hiring loop debrief')).toBeTruthy();
  });
  // The header's "Dismiss failures" clears the WHOLE history via `api_llm_activity_dismiss`
  // (no id); per-row Dismiss (specs/0063 W3 Task 6) uses `api_llm_activity_dismiss_task`
  // to remove only that one record. This test covers both: per-row Retry by numeric task
  // id, and the header dismiss clearing everything.
  it('queue panel: per-row Retry dispatches by numeric task id, dismiss still clears the lot', () => {
    backlog.view = { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 } as typeof backlog.view;
    Object.assign(llm, {
      hasFailure: true,
      history: [{ id: 7, kind: 'prepBrief', label: 'Prep brief — Q3 planning', error: 'Ollama unreachable', meetingId: 'q3', outcome: { type: 'failed', error: 'Ollama unreachable' } }],
    });
    render(<TransportRail />);
    fireEvent.click(screen.getByRole('button', { name: /queue/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(invokeMock).toHaveBeenCalledWith('api_llm_activity_retry_task', { taskId: 7 });
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss failures' }));
    expect(llm.dismiss).toHaveBeenCalledTimes(1);
  });
});
