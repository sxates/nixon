import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingSpeaker } from '@/types';

// specs/0064 item 4 (owner feedback, folded in via controller Ruling R6) — the Speakers
// section had no top padding and its title floated above the row list instead of inside
// a card, unlike the Participants box directly above it. This locks the card treatment
// while keeping the early `return null` paths (recording / no speakers) empty — a
// notes-only meeting must not show an empty box.

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
    speakerPhotos: new Map(),
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
  invoke.mockResolvedValue({ transcripts: [], total_count: 0, has_more: false });
});

describe('SpeakerLegend — boxed like Participants (specs/0064 item 4)', () => {
  it('renders the Speakers title inside a card with the participants box treatment', () => {
    const speakers = [speaker('local', 'You', true), speaker('spk_1', 'Speaker 2')];
    render(<SpeakerLegend meetingId="meeting-1" controller={makeController(speakers)} />);

    const title = screen.getByText('Speakers');
    const card = title.closest('.border-border');
    expect(card).not.toBeNull();
    expect(card).toHaveClass('bg-card');
  });

  it('renders nothing for a meeting with no speakers, so no empty card appears', () => {
    const { container } = render(
      <SpeakerLegend meetingId="meeting-1" controller={makeController([])} />,
    );

    expect(container.firstChild).toBeNull();
  });

  it('renders nothing while recording, even with speakers present', () => {
    const speakers = [speaker('local', 'You', true)];
    const { container } = render(
      <SpeakerLegend
        meetingId="meeting-1"
        controller={makeController(speakers)}
        isRecording
      />,
    );

    expect(container.firstChild).toBeNull();
  });
});
