import { describe, it, expect, vi, beforeEach } from 'vitest';

const h = vi.hoisted(() => ({ invokeMock: vi.fn(), toastError: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invokeMock }));
vi.mock('sonner', () => ({ toast: { error: h.toastError } }));

import { parseMeetingLink, openMeetingMoment } from '../meetingLink';

describe('parseMeetingLink', () => {
  it('parses a meeting link with a time', () => {
    expect(parseMeetingLink('nixon://meeting/abc?t=754.2')).toEqual({ meetingId: 'abc', t: 754.2 });
  });
  it('accepts the dev scheme, time optional', () => {
    expect(parseMeetingLink('nixon-dev://meeting/abc')).toEqual({ meetingId: 'abc', t: null });
  });
  it('decodes the id', () => {
    expect(parseMeetingLink('nixon://meeting/a%20b')?.meetingId).toBe('a b');
  });
  it.each(['?t=-3', '?t=NaN', '?t=', '?t=abc', '?t=Infinity'])('drops a bad time %s', (q) => {
    expect(parseMeetingLink(`nixon://meeting/abc${q}`)).toEqual({ meetingId: 'abc', t: null });
  });
  it.each([
    'nixon://meeting/',
    'nixon://meeting',
    'nixon://meeting/a/b',
    'nixon://record',
    'nixon://record/abc',
    'https://meeting/abc',
    'not a url',
  ])('rejects %s', (url) => {
    expect(parseMeetingLink(url)).toBeNull();
  });
});

describe('openMeetingMoment', () => {
  const router = { push: vi.fn() };
  beforeEach(() => vi.clearAllMocks());

  it('pushes the segment route when the moment resolves', async () => {
    h.invokeMock.mockResolvedValue({ exists: true, segmentId: 's 1' });
    await openMeetingMoment(router, 'abc', 60);
    expect(h.invokeMock).toHaveBeenCalledWith('api_resolve_meeting_link', { meetingId: 'abc', t: 60 });
    expect(router.push).toHaveBeenCalledWith('/meeting-details?id=abc&segment=s%201');
  });

  it('pushes the plain route when no segment resolves', async () => {
    h.invokeMock.mockResolvedValue({ exists: true, segmentId: null });
    await openMeetingMoment(router, 'abc', null);
    expect(router.push).toHaveBeenCalledWith('/meeting-details?id=abc');
  });

  it('toasts and does not navigate when the meeting is gone', async () => {
    h.invokeMock.mockResolvedValue({ exists: false, segmentId: null });
    await openMeetingMoment(router, 'abc', 1);
    expect(router.push).not.toHaveBeenCalled();
    expect(h.toastError).toHaveBeenCalledWith("That meeting isn't in Nixon anymore.");
  });

  it('toasts when resolving fails', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    h.invokeMock.mockRejectedValue(new Error('boom'));
    await openMeetingMoment(router, 'abc', 1);
    expect(router.push).not.toHaveBeenCalled();
    expect(h.toastError).toHaveBeenCalledWith("Nixon couldn't open that link.");
  });
});
