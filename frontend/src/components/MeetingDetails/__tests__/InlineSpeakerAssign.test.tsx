import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { InlineSpeakerAssign } from '@/components/MeetingDetails/InlineSpeakerAssign';
import type { MeetingAttendee, Person } from '@/types';

// specs/0019 WS2.1 (note 3) — a speaker can be named directly from a transcript line.
// These lock: (a) with no pickable identities the name is plain text (no affordance),
// (b) with people the name becomes a button that opens a picker, and picking a person
// calls onAssignPerson with the segment's speakerKey.

const PEOPLE: Person[] = [
  { id: 'p3', displayName: 'Priya Patel', role: 'Design', email: 'priya@acme.io', notes: null, voiceprintOptOut: false, starred: false, createdAt: '', updatedAt: '' },
];
const ATTENDEES: MeetingAttendee[] = [
  { name: 'Jordan Lee', email: 'jordan@acme.io', isCurrentUser: false },
];

describe('InlineSpeakerAssign (WS2.1)', () => {
  it('renders the speaker name as plain text when there is nothing to assign', () => {
    render(
      <InlineSpeakerAssign
        speakerKey="spk_0"
        speakerName="Speaker 1"
        attendees={[]}
        people={[]}
        onAssignAttendee={vi.fn()}
        onAssignPerson={vi.fn()}
      />,
    );
    // No button affordance — just the label.
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText('Speaker 1')).toBeInTheDocument();
  });

  it('assigns the speaker to a picked person via its speakerKey', async () => {
    const onAssignPerson = vi.fn().mockResolvedValue(undefined);
    render(
      <InlineSpeakerAssign
        speakerKey="spk_0"
        speakerName="Speaker 1"
        attendees={ATTENDEES}
        people={PEOPLE}
        onAssignAttendee={vi.fn()}
        onAssignPerson={onAssignPerson}
      />,
    );

    // The name is now an affordance.
    fireEvent.click(screen.getByRole('button', { name: /Speaker 1/ }));

    // Picker opens with the person; click them.
    const priya = await screen.findByText('Priya Patel');
    fireEvent.click(priya);

    await waitFor(() =>
      expect(onAssignPerson).toHaveBeenCalledWith('spk_0', expect.objectContaining({ id: 'p3' })),
    );
  });

  it('assigns a calendar attendee via its speakerKey', async () => {
    const onAssignAttendee = vi.fn().mockResolvedValue(undefined);
    render(
      <InlineSpeakerAssign
        speakerKey="spk_1"
        speakerName="Speaker 2"
        attendees={ATTENDEES}
        people={[]}
        onAssignAttendee={onAssignAttendee}
        onAssignPerson={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: /Speaker 2/ }));
    fireEvent.click(await screen.findByText('Jordan Lee'));

    await waitFor(() =>
      expect(onAssignAttendee).toHaveBeenCalledWith('spk_1', {
        name: 'Jordan Lee',
        email: 'jordan@acme.io',
      }),
    );
  });
});
