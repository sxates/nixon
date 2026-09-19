import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

// specs/0064 W2 — a suggestion the backend already applied must not come back as a
// "Looks like …" chip for the user to click. The backend decides (well-trained voiceprint,
// or a voice recognized across enough prior meetings) and applies it; the hook's job is to
// keep it out of the chip map and to reload the speakers so the applied name is visible.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { useSpeakers } from '@/hooks/useSpeakers';
import type { SpeakerSuggestion } from '@/types';

const suggestion = (
  speakerKey: string,
  autoLabel: boolean,
): SpeakerSuggestion => ({
  speakerKey,
  suggestedName: 'Priya',
  suggestedEmail: null,
  suggestedPersonId: 'person-1',
  confidence: 0.91,
  basis: autoLabel
    ? "recognized Priya's well-trained voiceprint"
    : "matched Priya's voice from 2 prior meetings",
  autoLabel,
});

/** Speakers as the backend returns them: spk_1 named by the auto-label, spk_2 not. */
const speakerRows = [
  { speakerKey: 'spk_1', displayName: 'Priya', isLocal: false },
  { speakerKey: 'spk_2', displayName: 'Speaker 2', isLocal: false },
];

function mockBackend(suggestions: SpeakerSuggestion[]) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_get_speaker_suggestions':
        return suggestions;
      case 'api_get_meeting_speakers':
        return speakerRows;
      case 'api_list_people_ranked':
        return [];
      case 'api_get_meeting_attendees':
        return { attendees: [], suggestion: null };
      default:
        return null;
    }
  });
}

const callsTo = (cmd: string) =>
  invokeMock.mock.calls.filter(([name]) => name === cmd).length;

describe('useSpeakers — auto-labeled suggestions (specs/0064 W2)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    sessionStorage.clear();
  });

  it('keeps an auto-labeled suggestion out of the chip map', async () => {
    mockBackend([suggestion('spk_1', true), suggestion('spk_2', false)]);

    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));
    await waitFor(() =>
      expect(result.current.crossMeetingSuggestions.size).toBeGreaterThan(0),
    );

    expect([...result.current.crossMeetingSuggestions.keys()]).toEqual(['spk_2']);
  });

  it('reloads the speakers once so the applied name shows', async () => {
    mockBackend([suggestion('spk_1', true)]);

    renderHook(() => useSpeakers({ meetingId: 'm1' }));

    // One load from the initial refresh, one more because an auto-label landed.
    await waitFor(() => expect(callsTo('api_get_meeting_speakers')).toBe(2));

    // …and it settles there: re-fetching the same auto-label must not loop.
    await new Promise((r) => setTimeout(r, 50));
    expect(callsTo('api_get_meeting_speakers')).toBe(2);
  });

  it('does not reload when nothing was auto-labeled', async () => {
    mockBackend([suggestion('spk_2', false)]);

    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    await new Promise((r) => setTimeout(r, 50));

    expect(callsTo('api_get_meeting_speakers')).toBe(1);
  });
});
