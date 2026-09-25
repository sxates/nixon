import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { AudioSetupResolved } from '@/hooks/useAudioSetup';
import type { MeetingSpeaker } from '@/types';

// specs/0078 W3 — "This is me" / "This isn't me". Locks the show/hide rules, that the
// legend chip's and the transcript name's popovers call the controller, and the room
// hint line.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { SpeakerLegend } from '@/components/MeetingDetails/SpeakerLegend';
import { InlineSpeakerAssign } from '@/components/MeetingDetails/InlineSpeakerAssign';
import {
  ownerActionFor,
  type OwnerActionContext,
} from '@/components/MeetingDetails/SpeakerOwnerAction';

const sp = (speakerKey: string, displayName: string, isLocal = false): MeetingSpeaker => ({
  speakerKey,
  displayName,
  isLocal,
});

function makeController(
  speakers: MeetingSpeaker[],
  resolved: AudioSetupResolved | null,
): UseSpeakersReturn {
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
    audioSetup: { override: 'auto', resolved },
    markAsMe: vi.fn().mockResolvedValue(undefined),
    unmarkMe: vi.fn().mockResolvedValue(undefined),
  };
}

function openChip(name: string) {
  fireEvent.click(screen.getByRole('button', { name: new RegExp(`^${name}`) }));
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue({ transcripts: [], total_count: 0, has_more: false });
});

describe('ownerActionFor (specs/0078)', () => {
  const ctx = (resolved: AudioSetupResolved | null, hasLocalSpeaker: boolean) => ({
    resolved,
    hasLocalSpeaker,
  });

  it('offers "This is me" on numbered speakers in a room meeting, even with a You', () => {
    expect(ownerActionFor('spk_0', false, ctx('room', true))).toBe('mark');
    expect(ownerActionFor('spk_0', false, ctx('hybrid', false))).toBe('mark');
  });

  it('offers "This is me" in any meeting that has no You yet', () => {
    expect(ownerActionFor('spk_0', false, ctx(null, false))).toBe('mark');
    expect(ownerActionFor('spk_0', false, ctx('call', false))).toBe('mark');
  });

  it('hides "This is me" in a call meeting that already has a You', () => {
    expect(ownerActionFor('spk_0', false, ctx('call', true))).toBeNull();
    expect(ownerActionFor('spk_0', false, ctx(null, true))).toBeNull();
  });

  it('never offers anything on "Unknown speaker"', () => {
    expect(ownerActionFor('unknown', false, ctx('room', false))).toBeNull();
  });

  it('offers "This isn\'t me" on You only in room/hybrid meetings', () => {
    expect(ownerActionFor('local', true, ctx('room', true))).toBe('unmark');
    expect(ownerActionFor('local', true, ctx('hybrid', true))).toBe('unmark');
    expect(ownerActionFor('local', true, ctx('call', true))).toBeNull();
    expect(ownerActionFor('local', true, ctx(null, true))).toBeNull();
    // A transcript line keyed `local` with no speakers row: nothing to unmark.
    expect(ownerActionFor('local', false, ctx('room', false))).toBeNull();
  });
});

describe('SpeakerLegend owner actions (specs/0078)', () => {
  it('"This is me" in a room meeting calls markAsMe with the chip\'s keys', () => {
    const c = makeController([sp('spk_0', 'Speaker 1'), sp('spk_1', 'Speaker 2')], 'room');
    render(<SpeakerLegend meetingId="m1" controller={c} />);
    openChip('Speaker 1');
    fireEvent.click(screen.getByRole('button', { name: 'This is me' }));
    expect(c.markAsMe).toHaveBeenCalledWith(['spk_0']);
  });

  it('"This isn\'t me" on You in a room meeting calls unmarkMe', () => {
    const c = makeController([sp('local', 'You', true), sp('spk_1', 'Speaker 2')], 'room');
    render(<SpeakerLegend meetingId="m1" controller={c} />);
    openChip('You');
    fireEvent.click(screen.getByRole('button', { name: "This isn't me" }));
    expect(c.unmarkMe).toHaveBeenCalledTimes(1);
  });

  it('offers neither action in a call meeting that has a You', () => {
    const c = makeController([sp('local', 'You', true), sp('spk_1', 'Speaker 2')], 'call');
    render(<SpeakerLegend meetingId="m1" controller={c} />);
    openChip('Speaker 2');
    expect(screen.queryByRole('button', { name: 'This is me' })).toBeNull();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' });
    openChip('You');
    expect(screen.queryByRole('button', { name: "This isn't me" })).toBeNull();
  });

  it('shows the room hint only for a room meeting with no You', () => {
    const hint = /Recorded in a room/;
    const { unmount } = render(
      <SpeakerLegend meetingId="m1" controller={makeController([sp('spk_0', 'Speaker 1')], 'room')} />,
    );
    expect(screen.getByText(hint)).toBeInTheDocument();
    unmount();

    render(
      <SpeakerLegend
        meetingId="m1"
        controller={makeController([sp('local', 'You', true), sp('spk_0', 'Speaker 1')], 'room')}
      />,
    );
    expect(screen.queryByText(hint)).toBeNull();
  });

  it('shows no hint for a call meeting', () => {
    render(
      <SpeakerLegend meetingId="m1" controller={makeController([sp('spk_0', 'Speaker 1')], 'call')} />,
    );
    expect(screen.queryByText(/Recorded in a room/)).toBeNull();
  });
});

describe('InlineSpeakerAssign owner action (specs/0078)', () => {
  const owner = (resolved: AudioSetupResolved | null, hasLocalSpeaker: boolean): OwnerActionContext => ({
    resolved,
    hasLocalSpeaker,
    markAsMe: vi.fn().mockResolvedValue(undefined),
    unmarkMe: vi.fn().mockResolvedValue(undefined),
  });

  it('makes the name an affordance for "This is me" even with no people to pick', () => {
    const o = owner('room', false);
    render(
      <InlineSpeakerAssign
        speakerKey="spk_0"
        speakerName="Speaker 1"
        attendees={[]}
        people={[]}
        onAssignAttendee={vi.fn()}
        onAssignPerson={vi.fn()}
        owner={o}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Speaker 1/ }));
    fireEvent.click(screen.getByRole('button', { name: 'This is me' }));
    expect(o.markAsMe).toHaveBeenCalledWith(['spk_0']);
  });

  it('stays plain text when no owner action applies and nothing to pick', () => {
    render(
      <InlineSpeakerAssign
        speakerKey="spk_0"
        speakerName="Speaker 1"
        attendees={[]}
        people={[]}
        onAssignAttendee={vi.fn()}
        onAssignPerson={vi.fn()}
        owner={owner('call', true)}
      />,
    );
    expect(screen.queryByRole('button')).toBeNull();
  });
});
