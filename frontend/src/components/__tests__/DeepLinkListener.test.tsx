import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, waitFor, act } from '@testing-library/react';

// specs/0087 — nixon:// links only navigate. The one command they may call is the
// read-only resolver.

const h = vi.hoisted(() => ({
  pushMock: vi.fn(),
  toastError: vi.fn(),
  invokeMock: vi.fn(),
  getCurrentMock: vi.fn(),
  onOpenUrlMock: vi.fn(),
  openUrlHandler: null as null | ((urls: string[]) => void),
}));

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: h.pushMock }) }));
vi.mock('sonner', () => ({ toast: { error: h.toastError } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invokeMock }));
vi.mock('@tauri-apps/plugin-deep-link', () => ({
  getCurrent: h.getCurrentMock,
  onOpenUrl: h.onOpenUrlMock,
}));

import { DeepLinkListener } from '../DeepLinkListener';

const resolve = (exists: boolean, segmentId: string | null) =>
  h.invokeMock.mockResolvedValue({ exists, segmentId });

beforeEach(() => {
  vi.clearAllMocks();
  h.openUrlHandler = null;
  h.getCurrentMock.mockResolvedValue(null);
  h.onOpenUrlMock.mockImplementation(async (cb: (urls: string[]) => void) => {
    h.openUrlHandler = cb;
    return () => {};
  });
});

describe('DeepLinkListener', () => {
  it('opens a cold URL from getCurrent at the segment', async () => {
    resolve(true, 's1');
    h.getCurrentMock.mockResolvedValue(['nixon://meeting/abc?t=60']);
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.pushMock).toHaveBeenCalledWith('/meeting-details?id=abc&segment=s1'));
    expect(h.invokeMock).toHaveBeenCalledWith('api_resolve_meeting_link', { meetingId: 'abc', t: 60 });
  });

  it('opens a warm URL from onOpenUrl', async () => {
    resolve(true, 's1');
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://meeting/abc?t=60']));
    await waitFor(() => expect(h.pushMock).toHaveBeenCalledWith('/meeting-details?id=abc&segment=s1'));
  });

  it('opens the meeting alone when no segment resolves, without a toast', async () => {
    resolve(true, null);
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://meeting/abc']));
    await waitFor(() => expect(h.pushMock).toHaveBeenCalledWith('/meeting-details?id=abc'));
    expect(h.toastError).not.toHaveBeenCalled();
  });

  it('says so when the meeting is gone', async () => {
    resolve(false, null);
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://meeting/abc']));
    await waitFor(() => expect(h.toastError).toHaveBeenCalledWith("That meeting isn't in Nixon anymore."));
    expect(h.pushMock).not.toHaveBeenCalled();
  });

  it('says so for an unparseable URL and never calls the backend', async () => {
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://record']));
    await waitFor(() => expect(h.toastError).toHaveBeenCalledWith("Nixon couldn't open that link."));
    expect(h.pushMock).not.toHaveBeenCalled();
    expect(h.invokeMock).not.toHaveBeenCalled();
  });

  it('navigates once when both paths deliver the same URL', async () => {
    resolve(true, 's1');
    h.getCurrentMock.mockResolvedValue(['nixon://meeting/abc?t=60']);
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://meeting/abc?t=60']));
    await waitFor(() => expect(h.pushMock).toHaveBeenCalled());
    expect(h.pushMock).toHaveBeenCalledTimes(1);
  });

  it('only ever invokes the resolver', async () => {
    resolve(true, 's1');
    render(<DeepLinkListener />);
    await waitFor(() => expect(h.openUrlHandler).not.toBeNull());
    await act(async () => h.openUrlHandler!(['nixon://meeting/abc?t=1', 'nixon://record']));
    await waitFor(() => expect(h.pushMock).toHaveBeenCalled());
    for (const call of h.invokeMock.mock.calls) expect(call[0]).toBe('api_resolve_meeting_link');
  });
});
