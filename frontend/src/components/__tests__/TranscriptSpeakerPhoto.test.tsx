import React from 'react';
import { render, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import {
  VirtualizedTranscriptView,
  type InlineSpeakerAssignment,
} from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import { buildSpeakerPhotoMap } from '@/lib/speaker-photos';
import { speakerChipProps } from '@/components/MeetingDetails/speaker-chip-props';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingAttendee, MeetingSpeaker, Person, TranscriptSegmentData } from '@/types';

// Owner feedback: Google directory photos were on people/attendee chips but the transcript's
// run-header avatar always showed initials. The meeting view now builds ONE speakerKey →
// photo map and each run header looks its speaker up in it.

const PHOTO_A = 'data:image/png;base64,QUFBQQ==';
const PHOTO_B = 'data:image/png;base64,QkJCQg==';
const PHOTO_SELF = 'data:image/png;base64,U0VMRg==';

function speaker(over: Partial<MeetingSpeaker>): MeetingSpeaker {
  return { speakerKey: 'spk_0', displayName: 'Speaker', isLocal: false, ...over };
}
function person(over: Partial<Person>): Person {
  return {
    id: 'p-x',
    email: null,
    displayName: 'Someone',
    role: null,
    notes: null,
    voiceprintOptOut: false,
    starred: false,
    createdAt: '',
    updatedAt: '',
    ...over,
  };
}
function attendee(over: Partial<MeetingAttendee>): MeetingAttendee {
  return { name: 'A', email: 'a@example.test', isCurrentUser: false, ...over };
}

describe('buildSpeakerPhotoMap', () => {
  it('resolves by linked person, then by email (people or attendees), then self for "You"', () => {
    const map = buildSpeakerPhotoMap(
      [
        speaker({ speakerKey: 'spk_0', personId: 'p-1' }),
        speaker({ speakerKey: 'spk_1', email: ' Bea@Example.test ' }),
        speaker({ speakerKey: 'spk_2', email: 'nobody@example.test' }),
        speaker({ speakerKey: 'local', displayName: 'You', isLocal: true }),
      ],
      [
        attendee({ email: 'bea@example.test', photoDataUri: PHOTO_B }),
        attendee({ email: 'me@example.test', isCurrentUser: true, photoDataUri: PHOTO_SELF }),
      ],
      [person({ id: 'p-1', email: 'ada@example.test', photoDataUri: PHOTO_A })],
    );
    expect(map.get('spk_0')).toBe(PHOTO_A);
    expect(map.get('spk_1')).toBe(PHOTO_B);
    expect(map.has('spk_2')).toBe(false);
    expect(map.get('local')).toBe(PHOTO_SELF);
  });
});

const SEGMENTS: TranscriptSegmentData[] = [
  { id: 's1', timestamp: 0, text: 'hello there', speaker: 'spk_0', speakerName: 'Ada Lovelace' },
  { id: 's2', timestamp: 3, text: 'hi', speaker: 'spk_1', speakerName: 'Grace Hopper' },
];

function assignment(photos: Map<string, string>): InlineSpeakerAssignment {
  return {
    attendees: [],
    people: [],
    onAssignAttendee: vi.fn(),
    onAssignPerson: vi.fn(),
    speakers: [],
    onReassignSegment: vi.fn(),
    onReassignSegments: vi.fn(),
    onCreateSpeaker: vi.fn(),
    speakerPhotos: photos,
  };
}

function header(id: string) {
  return document.getElementById(`segment-${id}`)!;
}

describe('transcript run-header avatar', () => {
  it('shows the photo for a speaker that has one and initials for one that does not', () => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={SEGMENTS}
          disableAutoScroll
          assignment={assignment(new Map([['spk_0', PHOTO_A]]))}
        />
      </TooltipProvider>,
    );
    const ada = header('s1');
    expect(ada.querySelector('img')?.getAttribute('src')).toBe(PHOTO_A);
    const grace = header('s2');
    expect(grace.querySelector('img')).toBeNull();
    expect(grace.textContent).toContain('GH');
  });

  it('falls back to initials when the photo fails to load', () => {
    render(
      <TooltipProvider>
        <VirtualizedTranscriptView
          segments={SEGMENTS}
          disableAutoScroll
          assignment={assignment(new Map([['spk_0', PHOTO_A]]))}
        />
      </TooltipProvider>,
    );
    fireEvent.error(header('s1').querySelector('img')!);
    expect(header('s1').querySelector('img')).toBeNull();
    expect(header('s1').textContent).toContain('AL');
  });
});

describe('speakerChipProps — legend photo', () => {
  it("takes the group's photo from controller.speakerPhotos (any member key), else null", () => {
    const primary = speaker({ speakerKey: 'spk_0', displayName: 'Ada Lovelace' });
    const other = speaker({ speakerKey: 'spk_3', displayName: 'Ada Lovelace' });
    const controller = {
      speakers: [primary, other],
      speakerPhotos: new Map([['spk_3', PHOTO_A]]),
      crossMeetingSuggestions: new Map(),
    } as unknown as UseSpeakersReturn;
    const ctx = { allSpeakers: [primary], controller, onPersonSaved: vi.fn() };
    const merged = { primary, members: [primary, other], keys: ['spk_0', 'spk_3'] };
    expect(speakerChipProps(merged, ctx).photoDataUri).toBe(PHOTO_A);
    const alone = { primary, members: [primary], keys: ['spk_0'] };
    expect(speakerChipProps(alone, ctx).photoDataUri).toBeNull();
  });
});
