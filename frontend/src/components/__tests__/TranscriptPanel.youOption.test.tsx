import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingSpeaker, TranscriptSegmentData } from '@/types';

// specs/0061 W4 part B (task 2) — Task 1 made reassigning a line to the owner work
// backend-side even with no `local` speakers row yet (ensure_local_speaker upserts it).
// This locks the frontend half: the reassignment menus must offer "You" as a target
// even when `speakersController.speakers` has no `local` entry, so the owner isn't
// stuck fighting a menu that doesn't list them.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
// The button group pulls in config/backlog/diarization context this test doesn't
// exercise; stub it out so only the reassign-menu wiring under test is live.
vi.mock('../MeetingDetails/TranscriptButtonGroup', () => ({
  TranscriptButtonGroup: () => null,
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  // `view` is required since specs/0071 W3 — the panel asks whether its own meeting is
  // being processed so it can say so, which the real provider has always supplied.
  useBacklog: () => ({
    view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    enqueueMeeting: vi.fn(),
  }),
}));

import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';

const SEGMENTS: TranscriptSegmentData[] = [
  { id: 'seg-1', timestamp: 0, text: 'line one', speaker: 'spk_0', speakerName: 'Speaker 1' },
  { id: 'seg-2', timestamp: 3, text: 'line two', speaker: 'spk_0', speakerName: 'Speaker 1' },
];

function makeSpeakersController(speakers: MeetingSpeaker[]): UseSpeakersReturn {
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
  };
}

function renderPanel(speakersController: UseSpeakersReturn) {
  return render(
    <TooltipProvider>
      <TranscriptPanel
        transcripts={[]}
        onCopyTranscript={vi.fn()}
        onOpenMeetingFolder={vi.fn().mockResolvedValue(undefined)}
        isRecording={false}
        usePagination
        segments={SEGMENTS}
        meetingId="meeting-1"
        speakersController={speakersController}
      />
    </TooltipProvider>,
  );
}

// Select the two lines, then open the "Reassign to…" floating menu — mirrors
// VirtualizedTranscriptView.spanReassign.test.tsx's openReassignMenu helper.
async function openReassignMenu() {
  const boxes = screen.getAllByRole('checkbox');
  fireEvent.click(boxes[0]);
  fireEvent.click(boxes[1], { shiftKey: true });
  const trigger = screen.getByRole('button', { name: /reassign to/i });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  return screen.findAllByRole('menuitem');
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
});

describe('TranscriptPanel — "You" is always a reassignment target (specs/0061 W4 part B)', () => {
  it('offers "You" first in the reassign menu even when no local speaker row exists yet', async () => {
    // The speaker list has no `local` row at all (never diarized as the owner yet).
    renderPanel(makeSpeakersController([{ speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false }]));

    const items = await openReassignMenu();
    expect(items[0]).toHaveTextContent('You');
  });

  it('does not duplicate "You" when a real local speaker row already exists', async () => {
    renderPanel(
      makeSpeakersController([
        { speakerKey: 'local', displayName: 'You', isLocal: true },
        { speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false },
      ]),
    );

    const items = await openReassignMenu();
    const youItems = items.filter((item) => item.textContent?.includes('You'));
    expect(youItems).toHaveLength(1);
  });
});
