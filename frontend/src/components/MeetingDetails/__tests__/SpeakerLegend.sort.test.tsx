import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingSpeaker, Transcript } from '@/types';

// specs/0064 W4 (owner feedback item 7) — the channel strip listed speakers in whatever
// order the DB returned them, so the share-of-talk column climbed and fell down the page.
// Channels now descend by share, and CH is numbered AFTER the sort so it reads 1..n down
// the column rather than preserving a meaningless original index.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { SpeakerLegend } from '@/components/MeetingDetails/SpeakerLegend';

function speaker(speakerKey: string, displayName: string, isLocal = false): MeetingSpeaker {
  return { speakerKey, displayName, isLocal };
}

function line(speakerKey: string, duration: number, i: number): Transcript {
  return {
    id: `${speakerKey}-${i}`,
    text: 'x',
    timestamp: '',
    speaker: speakerKey,
    duration,
  } as Transcript;
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
    setAudioSetup: vi.fn().mockResolvedValue({ started: true, alreadyRunning: false }),
    refetchAudioSetup: vi.fn().mockResolvedValue(undefined),
    markAsMe: vi.fn().mockResolvedValue(undefined),
    unmarkMe: vi.fn().mockResolvedValue(undefined),
  };
}

beforeEach(() => {
  invoke.mockReset();
  // No whole-meeting map, so the legend falls back to the transcripts it is handed.
  invoke.mockResolvedValue({ transcripts: [], total_count: 0, has_more: false });
});

describe('SpeakerLegend — ordered by share of talk (specs/0064 W4)', () => {
  it('puts the biggest talker first and numbers the channels top-down', () => {
    // Quietest first in the speakers list, so a passing test cannot be the input order.
    const speakers = [
      speaker('local', 'You', true),
      speaker('spk_2', 'Tomas'),
      speaker('spk_1', 'Maya'),
    ];
    const transcripts = [
      line('local', 10, 0),
      line('spk_2', 20, 1),
      line('spk_1', 30, 2),
    ];

    render(
      <SpeakerLegend
        meetingId="meeting-1"
        controller={makeController(speakers)}
        transcripts={transcripts}
        onSelectSpeaker={vi.fn()}
      />,
    );

    const rows = screen.getAllByRole('row').slice(1); // drop the header row
    const names = rows.map((r) => within(r).getByRole('button', { name: /filter transcript to/i }).getAttribute('aria-label'));
    expect(names).toEqual([
      'Filter transcript to Maya',
      'Filter transcript to Tomas',
      'Filter transcript to You',
    ]);

    // CH is a rank now, so it always reads 1, 2, 3 down the column.
    const channels = rows.map((r) => within(r).getByText(/^[0-9]+$/).textContent);
    expect(channels).toEqual(['1', '2', '3']);
  });

  it('keeps the order stable when two speakers have identical talk time', () => {
    const speakers = [speaker('spk_2', 'Bea'), speaker('spk_1', 'Ada')];
    const transcripts = [line('spk_2', 10, 0), line('spk_1', 10, 1)];

    render(
      <SpeakerLegend
        meetingId="meeting-1"
        controller={makeController(speakers)}
        transcripts={transcripts}
        onSelectSpeaker={vi.fn()}
      />,
    );

    const rows = screen.getAllByRole('row').slice(1);
    const names = rows.map((r) => within(r).getByRole('button', { name: /filter transcript to/i }).getAttribute('aria-label'));
    expect(names).toEqual(['Filter transcript to Ada', 'Filter transcript to Bea']);
  });
});
