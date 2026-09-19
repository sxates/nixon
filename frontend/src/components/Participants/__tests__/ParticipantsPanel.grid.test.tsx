import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// specs/0064 W4 (owner feedback item 3) — two complaints about the roster, one layout and
// one behavioural:
//
//   "the '…' and 'x' buttons appear on hover, but do not disappear after the mouse moves
//    away… those hidden buttons also push the names far apart. Let's arrange the
//    participants box into columns."
//
// The wrap became an auto-fit grid, so each name sits in a fixed cell and the reserved
// action width can no longer separate them. The lingering was `group-focus-within`: a click
// leaves focus on the trigger, so visibility ended when focus moved rather than when the
// pointer left.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { ParticipantsPanel } from '@/components/Participants/ParticipantsPanel';

const participants = [
  { personId: 'p1', displayName: 'Maya Okafor', email: 'maya@example.com', role: null, source: 'manual' },
  { personId: 'p2', displayName: 'Tomas Lindqvist', email: 'tomas@example.com', role: null, source: 'manual' },
];

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_get_meeting_participants') return participants;
    if (cmd === 'api_list_people_ranked') return [];
    return [];
  });
});

describe('ParticipantsPanel — column layout (specs/0064 W4)', () => {
  it('lays the roster out as an auto-fit grid rather than a ragged wrap', async () => {
    render(<ParticipantsPanel meetingId="m1" />);
    await waitFor(() => expect(screen.getByText('Maya Okafor')).toBeInTheDocument());

    const list = screen.getByTestId('participants-list');
    expect(list.className).toContain('grid');
    expect(list.className).not.toContain('flex-wrap');
    // Auto-fit with a minimum column width is what makes "four across at full width" hold
    // without a breakpoint ladder.
    expect(list.className).toContain('auto-fit');
  });

  it('keeps the roster single-column in the compact record-header popover', async () => {
    render(<ParticipantsPanel meetingId="m1" variant="compact" />);
    await waitFor(() => expect(screen.getByText('Maya Okafor')).toBeInTheDocument());

    const list = screen.getByTestId('participants-list');
    expect(list.className).toContain('grid-cols-1');
  });
});

describe('ParticipantChip — hover actions that let go (specs/0064 W4)', () => {
  it('does not keep the actions visible once focus lands on them from a click', async () => {
    render(<ParticipantsPanel meetingId="m1" />);
    await waitFor(() => expect(screen.getByText('Maya Okafor')).toBeInTheDocument());

    const remove = screen.getAllByRole('button', { name: /remove/i })[0]!;
    // `group-focus-within` is what made a clicked-then-unhovered chip stay lit: the trigger
    // keeps focus after the click. Visibility is hover + the button's own focus-visible +
    // an actually-open menu.
    expect(remove.className).not.toContain('group-focus-within');
    expect(remove.className).toContain('group-hover:opacity-100');
    expect(remove.className).toContain('focus-visible:opacity-100');
  });
});
