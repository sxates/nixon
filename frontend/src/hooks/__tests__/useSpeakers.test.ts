import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0019 WS2.2 (note 7) — assigning a speaker to a known person/attendee resolves it,
// so the lingering "looks like X" cross-meeting suggestion must auto-clear (no confirmation
// left hanging). This locks that behavior in the identity hook.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(() => () => {}),
  makeSafeUnlisten: vi.fn(() => () => {}),
}));
vi.mock('sonner', () => ({ toast: new Proxy({}, { get: () => vi.fn() }) }));

import { useSpeakers } from '@/hooks/useSpeakers';

const SUGGESTION = {
  speakerKey: 'spk_0',
  suggestedName: 'Priya Patel',
  suggestedEmail: 'priya@acme.io',
  confidence: 0.92,
  basis: 'voice',
};

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_get_speaker_suggestions':
        return Promise.resolve([SUGGESTION]);
      case 'api_get_meeting_speakers':
        return Promise.resolve([
          { speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false, email: null },
        ]);
      case 'api_list_people_ranked':
        return Promise.resolve([
          { id: 'p3', displayName: 'Priya Patel', role: 'Design', email: 'priya@acme.io', notes: null, voiceprintOptOut: false, starred: false, createdAt: '', updatedAt: '' },
        ]);
      case 'api_get_meeting_attendees':
        return Promise.resolve({ attendees: [], suggestion: null });
      default:
        return Promise.resolve(null);
    }
  });
});

describe('useSpeakers — assigning a person clears the cross-meeting suggestion (WS2.2)', () => {
  it('drops the "looks like X" suggestion for a speaker once it is assigned to a person', async () => {
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));

    // The suggestion shows up after the initial fetch.
    await waitFor(() => expect(result.current.crossMeetingSuggestions.has('spk_0')).toBe(true));

    await act(async () => {
      await result.current.assignPerson('spk_0', {
        id: 'p3',
        displayName: 'Priya Patel',
        role: 'Design',
        email: 'priya@acme.io',
        notes: null,
        voiceprintOptOut: false,
        starred: false,
        createdAt: '',
        updatedAt: '',
      });
    });

    // After assignment the suggestion is gone — no confirmation left hanging — even
    // though the backend fetch still returns it (dismissal wins).
    await waitFor(() => expect(result.current.crossMeetingSuggestions.has('spk_0')).toBe(false));
    expect(invokeMock).toHaveBeenCalledWith('api_assign_speaker_to_person', {
      meetingId: 'm1',
      speakerKey: 'spk_0',
      personId: 'p3',
    });
  });

  it('also clears it when assigning a calendar attendee', async () => {
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));
    await waitFor(() => expect(result.current.crossMeetingSuggestions.has('spk_0')).toBe(true));

    await act(async () => {
      await result.current.assignAttendee('spk_0', { name: 'Priya Patel', email: 'priya@acme.io' });
    });

    await waitFor(() => expect(result.current.crossMeetingSuggestions.has('spk_0')).toBe(false));
  });
});
