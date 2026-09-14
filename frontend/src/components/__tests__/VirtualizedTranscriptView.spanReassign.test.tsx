import React from 'react';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import {
  VirtualizedTranscriptView,
  type InlineSpeakerAssignment,
} from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { TranscriptSegmentData } from '@/types';

// specs/0039 WS2 (task 5) — span-level manual speaker correction. Locks the UI dispatch
// half of the acceptance criteria (the backend test covers the persistence half):
//  - selecting a contiguous run (click + shift-click) and reassigning calls
//    `onReassignSegments` (→ api_set_segment_speakers) with the right ids + key;
//  - the optimistic overlay applies immediately (labels flip before the write settles);
//  - a failed write reverts the overlay AND restores the selection so the user can retry;
//  - "New speaker…" mints a speaker (onCreateSpeaker), then reassigns the span to it.

// Three lines, all under one (wrong) speaker — the owner's "first stretch is wrong" case.
const SEGMENTS: TranscriptSegmentData[] = [
  { id: 'seg-1', timestamp: 0, text: 'line one', speaker: 'spk_0', speakerName: 'Speaker 1' },
  { id: 'seg-2', timestamp: 3, text: 'line two', speaker: 'spk_0', speakerName: 'Speaker 1' },
  { id: 'seg-3', timestamp: 6, text: 'line three', speaker: 'spk_0', speakerName: 'Speaker 1' },
];

function makeAssignment(
  over: Partial<InlineSpeakerAssignment> = {},
): InlineSpeakerAssignment {
  return {
    attendees: [],
    people: [],
    onAssignAttendee: vi.fn().mockResolvedValue(undefined),
    onAssignPerson: vi.fn().mockResolvedValue(undefined),
    speakers: [
      { speakerKey: 'spk_0', displayName: 'Speaker 1' },
      { speakerKey: 'spk_1', displayName: 'Speaker 2' },
    ],
    onReassignSegment: vi.fn().mockResolvedValue(undefined),
    onReassignSegments: vi.fn().mockResolvedValue(true),
    onCreateSpeaker: vi.fn().mockResolvedValue(null),
    ...over,
  };
}

function view(assignment: InlineSpeakerAssignment) {
  return (
    <TooltipProvider>
      <VirtualizedTranscriptView
        segments={SEGMENTS}
        disableAutoScroll
        assignment={assignment}
      />
    </TooltipProvider>
  );
}

// Select the contiguous span seg-1..seg-2 (click line 0, shift-click line 1).
function selectFirstTwoLines() {
  const boxes = screen.getAllByRole('checkbox');
  fireEvent.click(boxes[0]);
  fireEvent.click(boxes[1], { shiftKey: true });
}

// Open the "Reassign to…" menu from the floating action bar.
async function openReassignMenu() {
  const trigger = screen.getByRole('button', { name: /reassign to/i });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  return trigger;
}

// How many transcript lines currently render under a given speaker name (excludes menu
// items — those are queried by role separately).
function transcriptLabelCount(name: string): number {
  return screen
    .queryAllByText(name)
    .filter((el) => el.tagName === 'SPAN' && el.getAttribute('role') !== 'menuitem').length;
}

describe('VirtualizedTranscriptView — span reassignment (specs/0039 WS2)', () => {
  it('reassigns the selected span to an existing speaker with the right ids + key, optimistically', async () => {
    let resolve!: (v: boolean) => void;
    const onReassignSegments = vi.fn(
      () => new Promise<boolean>((r) => { resolve = r; }),
    );
    render(view(makeAssignment({ onReassignSegments })));

    selectFirstTwoLines();
    expect(screen.getByText(/2 lines selected/i)).toBeInTheDocument();

    await openReassignMenu();
    fireEvent.click(await screen.findByRole('menuitem', { name: /Speaker 2/i }));

    // Dispatched with exactly the selected ids + the chosen key.
    await waitFor(() =>
      expect(onReassignSegments).toHaveBeenCalledWith(['seg-1', 'seg-2'], 'spk_1'),
    );

    // Optimistic: the two selected lines already read "Speaker 2" while the write is
    // still in flight; the untouched third line stays "Speaker 1".
    await waitFor(() => expect(transcriptLabelCount('Speaker 2')).toBe(2));
    expect(transcriptLabelCount('Speaker 1')).toBe(1);

    resolve(true);
  });

  it('reverts the optimistic update and restores the selection when the write fails', async () => {
    const onReassignSegments = vi.fn().mockResolvedValue(false);
    render(view(makeAssignment({ onReassignSegments })));

    selectFirstTwoLines();
    await openReassignMenu();
    fireEvent.click(await screen.findByRole('menuitem', { name: /Speaker 2/i }));

    await waitFor(() => expect(onReassignSegments).toHaveBeenCalled());

    // On failure the labels revert to the server truth (all three "Speaker 1")…
    await waitFor(() => expect(transcriptLabelCount('Speaker 1')).toBe(3));
    expect(transcriptLabelCount('Speaker 2')).toBe(0);

    // …and the selection is restored so the user can retry.
    await waitFor(() =>
      expect(screen.getByText(/2 lines selected/i)).toBeInTheDocument(),
    );
    const checked = screen
      .getAllByRole('checkbox')
      .filter((el) => el.getAttribute('aria-checked') === 'true');
    expect(checked).toHaveLength(2);
  });

  it('shift-click EXTENDS the selection — earlier non-contiguous picks survive', () => {
    render(view(makeAssignment()));
    const boxes = screen.getAllByRole('checkbox');

    // Individually pick line 3 (seg-3), then line 1 (seg-1) — non-contiguous, anchor now line 1.
    fireEvent.click(boxes[2]);
    fireEvent.click(boxes[0]);

    // Shift-click line 2 (seg-2): the anchor→target range unions into the existing set,
    // so the earlier seg-3 pick is preserved (the old replace behaviour would drop it,
    // leaving only 2 selected).
    fireEvent.click(boxes[1], { shiftKey: true });

    expect(screen.getByText(/3 lines selected/i)).toBeInTheDocument();
    const checked = screen
      .getAllByRole('checkbox')
      .filter((el) => el.getAttribute('aria-checked') === 'true');
    expect(checked).toHaveLength(3);
  });

  it('mints a new speaker then reassigns the span to it', async () => {
    const onCreateSpeaker = vi
      .fn()
      .mockResolvedValue({ speakerKey: 'manual_abc', displayName: 'Dana' });
    const onReassignSegments = vi.fn().mockResolvedValue(true);
    render(view(makeAssignment({ onCreateSpeaker, onReassignSegments })));

    selectFirstTwoLines();
    await openReassignMenu();
    fireEvent.click(await screen.findByRole('menuitem', { name: /new speaker/i }));

    const dialog = await screen.findByRole('dialog');
    fireEvent.change(within(dialog).getByLabelText(/new speaker name/i), {
      target: { value: 'Dana' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /create & reassign/i }));

    await waitFor(() => expect(onCreateSpeaker).toHaveBeenCalledWith('Dana'));
    await waitFor(() =>
      expect(onReassignSegments).toHaveBeenCalledWith(['seg-1', 'seg-2'], 'manual_abc'),
    );
  });
});
