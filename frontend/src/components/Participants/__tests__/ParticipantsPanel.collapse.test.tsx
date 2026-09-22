import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Owner feedback 2026-09-21 — "the participants box gets really tall with several rows of
// participants. For meetings with 4+ participants, lets create a new collapsed version that
// summarizes number of participants with a few avatars… with an affordance to expand/collapse
// the whole list."
//
// The roster grid caps at ten and runs roughly four across (specs/0064 W4), so five people
// already cost two rows and ten cost three — all of it above the document tabs. Below four
// there is one row and nothing worth hiding.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { ParticipantsPanel } from '@/components/Participants/ParticipantsPanel';
import { participantSummaryText } from '@/components/Participants/ParticipantsSummary';

const NAMES = [
  'Maya Okafor',
  'Tomas Lindqvist',
  'Inès Marchetti',
  'Dana Whitfield',
  'Sam Okonjo',
];

function roster(n: number) {
  return NAMES.slice(0, n).map((displayName, i) => ({
    personId: `p${i}`,
    displayName,
    email: `p${i}@example.com`,
    role: null,
    source: 'manual' as const,
  }));
}

function mockRoster(n: number) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_get_meeting_participants') return roster(n);
    if (cmd === 'api_list_people_ranked') return [];
    return [];
  });
}

beforeEach(() => {
  invoke.mockReset();
});

describe('ParticipantsPanel — collapsed roster (owner feedback 2026-09-21)', () => {
  it('starts collapsed at four participants, showing a summary instead of the grid', async () => {
    mockRoster(4);
    render(<ParticipantsPanel meetingId="m1" />);

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Expand participants' })).toBeInTheDocument(),
    );
    expect(screen.queryByTestId('participants-list')).not.toBeInTheDocument();
    expect(
      screen.getByText('Maya Okafor, Tomas Lindqvist, Inès Marchetti and 1 other'),
    ).toBeInTheDocument();
  });

  it('stays open at three participants — one row is not worth a click', async () => {
    mockRoster(3);
    render(<ParticipantsPanel meetingId="m1" />);

    await waitFor(() => expect(screen.getByTestId('participants-list')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: /expand participants/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /collapse participants/i })).not.toBeInTheDocument();
  });

  it('expands to the full grid and collapses again', async () => {
    mockRoster(5);
    render(<ParticipantsPanel meetingId="m1" />);

    const expand = await screen.findByRole('button', { name: 'Expand participants' });
    await userEvent.click(expand);

    const list = await screen.findByTestId('participants-list');
    expect(list).toBeInTheDocument();
    NAMES.slice(0, 5).forEach((n) => expect(screen.getByText(n)).toBeInTheDocument());

    await userEvent.click(screen.getByRole('button', { name: 'Collapse participants' }));
    await waitFor(() =>
      expect(screen.queryByTestId('participants-list')).not.toBeInTheDocument(),
    );
  });

  it('the summary row itself expands the roster', async () => {
    mockRoster(5);
    render(<ParticipantsPanel meetingId="m1" />);

    await userEvent.click(await screen.findByRole('button', { name: /Show all/ }));
    expect(await screen.findByTestId('participants-list')).toBeInTheDocument();
  });

  // The popover on the record header is already a scrolling column in a 320px flyout the
  // user opened deliberately; hiding its contents behind another click would be absurd.
  it('never collapses the compact in-recording popover', async () => {
    mockRoster(5);
    render(<ParticipantsPanel meetingId="m1" variant="compact" />);

    await waitFor(() => expect(screen.getByTestId('participants-list')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: /expand participants/i })).not.toBeInTheDocument();
  });
});

describe('participantSummaryText', () => {
  it('names one', () => {
    expect(participantSummaryText(roster(1))).toBe('Maya Okafor');
  });

  it('joins two with "and"', () => {
    expect(participantSummaryText(roster(2))).toBe('Maya Okafor and Tomas Lindqvist');
  });

  it('names all three at the threshold boundary', () => {
    expect(participantSummaryText(roster(3))).toBe(
      'Maya Okafor, Tomas Lindqvist and Inès Marchetti',
    );
  });

  it('counts the rest past three, singular', () => {
    expect(participantSummaryText(roster(4))).toBe(
      'Maya Okafor, Tomas Lindqvist, Inès Marchetti and 1 other',
    );
  });

  it('counts the rest past three, plural', () => {
    expect(participantSummaryText(roster(5))).toBe(
      'Maya Okafor, Tomas Lindqvist, Inès Marchetti and 2 others',
    );
  });

  it('is empty for an empty roster', () => {
    expect(participantSummaryText([])).toBe('');
  });
});
