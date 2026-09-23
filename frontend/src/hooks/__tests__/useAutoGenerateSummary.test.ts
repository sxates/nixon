import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useAutoGenerateSummary } from '@/hooks/meeting-details/useAutoGenerateSummary';

const modelConfig = { provider: 'ollama', model: 'llama3' } as never;

const params = (over: Record<string, unknown> = {}) => ({
  shouldAutoGenerate: true,
  meetingId: 'meeting-1',
  folderPath: null,
  transcriptCount: 3,
  modelConfig,
  generateSummary: vi.fn().mockResolvedValue(undefined),
  isProcessingInBacklog: false,
  ...over,
});

describe('useAutoGenerateSummary — origin (specs/0063 W3 Task 6b)', () => {
  beforeEach(() => invokeMock.mockReset());

  it('declares the auto run as background, so it reaches the rail Queue', async () => {
    // The whole point of the flag: nobody asked for this summary, and the user can walk
    // away from the meeting while it runs. Without `background: true` the Rust side tags it
    // Foreground and `LlmActivityRegistry::view()` filters it out of the running list, so
    // the work is invisible the moment you navigate off the detail page.
    const p = params();
    renderHook(() => useAutoGenerateSummary(p));
    await waitFor(() => expect(p.generateSummary).toHaveBeenCalled());
    expect(p.generateSummary).toHaveBeenCalledWith('', { background: true });
  });

  it('still runs only once per meeting even as later transcript pages arrive', async () => {
    const p = params({ transcriptCount: 1 });
    const { rerender } = renderHook((props: typeof p) => useAutoGenerateSummary(props), {
      initialProps: p,
    });
    await waitFor(() => expect(p.generateSummary).toHaveBeenCalledTimes(1));
    rerender({ ...p, transcriptCount: 12 });
    rerender({ ...p, transcriptCount: 30 });
    await waitFor(() => expect(p.generateSummary).toHaveBeenCalledTimes(1));
  });

  it('does not generate at all when the flag is off', async () => {
    const p = params({ shouldAutoGenerate: false });
    renderHook(() => useAutoGenerateSummary(p));
    await new Promise((r) => setTimeout(r, 20));
    expect(p.generateSummary).not.toHaveBeenCalled();
  });

  it('waits rather than generating when no transcripts are loaded and no audio awaits', async () => {
    // WS7.3: the transcript count arrives a beat late. Generating here would summarize an
    // empty meeting; the effect must re-arm instead.
    const p = params({ transcriptCount: 0, folderPath: '/tmp/rec' });
    invokeMock.mockResolvedValue(false); // api_meeting_audio_available
    renderHook(() => useAutoGenerateSummary(p));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(p.generateSummary).not.toHaveBeenCalled();
  });

  it('proceeds for a record-only meeting whose audio still awaits transcription', async () => {
    const p = params({ transcriptCount: 0, folderPath: '/tmp/rec' });
    invokeMock.mockResolvedValue(true); // api_meeting_audio_available
    renderHook(() => useAutoGenerateSummary(p));
    await waitFor(() => expect(p.generateSummary).toHaveBeenCalled());
    expect(p.generateSummary).toHaveBeenCalledWith('', { background: true });
  });
});

describe('useAutoGenerateSummary — the deferred backlog owns the meeting', () => {
  beforeEach(() => invokeMock.mockReset());

  // Measured 2026-09-23: the page's gate saw an empty backlog at 03:31:32 and armed; the
  // drain picked the meeting up at 03:31:37; the page fired its own summary at 03:31:39
  // anyway. Two runs of the same meeting, and the second one stranded the page's progress.
  it('does not generate when the backlog is processing the meeting', async () => {
    const p = params({ isProcessingInBacklog: true });
    renderHook(() => useAutoGenerateSummary(p));
    await new Promise((r) => setTimeout(r, 20));
    expect(p.generateSummary).not.toHaveBeenCalled();
  });

  it('re-checks at fire time: a drain that starts during the audio probe wins', async () => {
    let resolveProbe: (v: boolean) => void = () => {};
    invokeMock.mockReturnValue(new Promise<boolean>((r) => { resolveProbe = r; }));
    const p = params({ transcriptCount: 0, folderPath: '/tmp/rec', isProcessingInBacklog: false });
    const { rerender } = renderHook((props: typeof p) => useAutoGenerateSummary(props), {
      initialProps: p,
    });
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    rerender({ ...p, isProcessingInBacklog: true });
    resolveProbe(true);
    await new Promise((r) => setTimeout(r, 20));
    expect(p.generateSummary).not.toHaveBeenCalled();
  });
});
