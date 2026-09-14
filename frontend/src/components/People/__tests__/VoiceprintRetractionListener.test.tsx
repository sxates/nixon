import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, act, waitFor } from '@testing-library/react';

// specs/0039 WS3 (task 9) — app-wide undo toast for voiceprint retraction. These lock:
//  - a `voiceprint-retracted` event shows a sonner toast with an Undo action;
//  - Undo restores EVERY quarantined sample id from the payload via
//    api_restore_voiceprint_sample;
//  - repeated events for the SAME person aggregate into one toast (stable id) whose Undo
//    restores the union of ids — no stacking of N identical toasts.

const { invokeMock, listeners, toastFn } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  // Capture the toast options so the test can invoke the Undo action.
  toastFn: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn((eventName: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(eventName, handler);
    return () => listeners.delete(eventName);
  }),
}));
vi.mock('sonner', () => ({
  toast: Object.assign(toastFn, {
    success: vi.fn(),
    error: vi.fn(),
    dismiss: vi.fn(),
  }),
}));

import VoiceprintRetractionListener from '@/components/People/VoiceprintRetractionListener';

interface ToastOpts {
  id?: string;
  action?: { label: string; onClick: () => void };
}

function fire(payload: {
  meetingId: string;
  personId: string;
  personName: string;
  quarantinedSampleIds: string[];
}) {
  act(() => {
    listeners.get('voiceprint-retracted')?.({ payload });
  });
}

/** Latest toast() title + options. */
function lastToast(): { title: unknown; opts: ToastOpts } {
  const call = toastFn.mock.calls.at(-1)!;
  return { title: call[0], opts: (call[1] ?? {}) as ToastOpts };
}

describe('VoiceprintRetractionListener (specs/0039 WS3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listeners.clear();
    invokeMock.mockResolvedValue(undefined);
  });

  it('shows an undo toast on a retraction and restores the sample ids', async () => {
    render(<VoiceprintRetractionListener />);

    fire({
      meetingId: 'm-1',
      personId: 'p-1',
      personName: 'Priya Patel',
      quarantinedSampleIds: ['vp-1', 'vp-2'],
    });

    const { title, opts } = lastToast();
    expect(String(title)).toContain('Priya Patel');
    expect(opts.action?.label).toBe('Undo');

    act(() => opts.action?.onClick());

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-1' });
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-2' });
    });
  });

  it('aggregates repeated events for the same person into one toast (stable id)', () => {
    render(<VoiceprintRetractionListener />);

    fire({ meetingId: 'm-1', personId: 'p-1', personName: 'Priya', quarantinedSampleIds: ['vp-1'] });
    const firstId = lastToast().opts.id;

    fire({ meetingId: 'm-1', personId: 'p-1', personName: 'Priya', quarantinedSampleIds: ['vp-2'] });
    const secondId = lastToast().opts.id;

    // Same stable toast id ⇒ the second event updates the same toast, not a new one.
    expect(firstId).toBeDefined();
    expect(secondId).toBe(firstId);
  });

  it('restores the union of ids from aggregated same-person events', async () => {
    render(<VoiceprintRetractionListener />);

    fire({ meetingId: 'm-1', personId: 'p-1', personName: 'Priya', quarantinedSampleIds: ['vp-1'] });
    fire({ meetingId: 'm-1', personId: 'p-1', personName: 'Priya', quarantinedSampleIds: ['vp-2', 'vp-3'] });

    act(() => lastToast().opts.action?.onClick());

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-1' });
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-2' });
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-3' });
    });
  });

  it('keeps the failed id retryable when one restore rejects (partial failure)', async () => {
    render(<VoiceprintRetractionListener />);

    fire({
      meetingId: 'm-1',
      personId: 'p-1',
      personName: 'Priya',
      quarantinedSampleIds: ['vp-1', 'vp-2', 'vp-3'],
    });

    // vp-2 rejects; vp-1 and vp-3 succeed — allSettled must not sink the successes.
    invokeMock.mockImplementation((_cmd: string, args: { sampleId: string }) =>
      args.sampleId === 'vp-2'
        ? Promise.reject(new Error('boom'))
        : Promise.resolve(undefined),
    );

    const callsBefore = toastFn.mock.calls.length;
    act(() => lastToast().opts.action?.onClick());

    // All three are attempted.
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-1' });
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-2' });
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-3' });
    });

    // The undo toast re-shows for the still-quarantined id, Undo still available.
    await waitFor(() => expect(toastFn.mock.calls.length).toBeGreaterThan(callsBefore));
    const reshown = lastToast();
    expect(reshown.opts.action?.label).toBe('Undo');

    // Retrying Undo restores ONLY the remaining failed id — the two successes are no
    // longer pending, so they aren't re-restored.
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(undefined);
    act(() => reshown.opts.action?.onClick());

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-2' }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-1' });
    expect(invokeMock).not.toHaveBeenCalledWith('api_restore_voiceprint_sample', { sampleId: 'vp-3' });
  });

  it('ignores events with no sample ids', () => {
    render(<VoiceprintRetractionListener />);
    fire({ meetingId: 'm-1', personId: 'p-1', personName: 'Priya', quarantinedSampleIds: [] });
    expect(toastFn).not.toHaveBeenCalled();
  });
});
