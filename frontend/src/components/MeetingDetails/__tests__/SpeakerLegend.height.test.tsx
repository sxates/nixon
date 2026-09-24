import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingSpeaker } from '@/types';

// specs/0061 W4 part B (task 2) — the channel strip's scroll container was capped at
// `max-h-32` (~4 rows), so a six-speaker meeting clipped and forced a scrollbar for a
// perfectly common case. This locks the taller cap.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { SpeakerLegend } from '@/components/MeetingDetails/SpeakerLegend';

function speaker(speakerKey: string, displayName: string, isLocal = false): MeetingSpeaker {
  return { speakerKey, displayName, isLocal };
}

function makeController(speakers: MeetingSpeaker[]): UseSpeakersReturn {
  return {
    speakers,
    attendees: [],
    people: [],
    suggestion: null,
    crossMeetingSuggestions: new Map(),
    dismissSuggestion: vi.fn(),
    isLoading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
    renameSpeaker: vi.fn().mockResolvedValue(undefined),
    assignAttendee: vi.fn().mockResolvedValue(undefined),
    assignPerson: vi.fn().mockResolvedValue(undefined),
    mergeSpeakers: vi.fn().mockResolvedValue(undefined),
    audioSetup: null,
    markAsMe: vi.fn().mockResolvedValue(undefined),
    unmarkMe: vi.fn().mockResolvedValue(undefined),
  };
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue({ transcripts: [], total_count: 0, has_more: false });
});

describe('SpeakerLegend — channel strip scroll cap (specs/0061 W4 part B)', () => {
  it('caps the scroll container at max-h-[15rem] so six rows fit without scrolling', () => {
    const speakers = [
      speaker('local', 'You', true),
      speaker('spk_1', 'Speaker 2'),
      speaker('spk_2', 'Speaker 3'),
      speaker('spk_3', 'Speaker 4'),
      speaker('spk_4', 'Speaker 5'),
      speaker('spk_5', 'Speaker 6'),
    ];
    const { container } = render(
      <SpeakerLegend meetingId="meeting-1" controller={makeController(speakers)} />,
    );

    const scrollContainer = container.querySelector('.overflow-y-auto');
    expect(scrollContainer).not.toBeNull();
    expect(scrollContainer).toHaveClass('max-h-[15rem]');
    expect(scrollContainer?.className).not.toMatch(/max-h-32\b/);
  });
});
