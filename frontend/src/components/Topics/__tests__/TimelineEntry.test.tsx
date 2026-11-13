import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@/lib/meetingLink', () => ({ openMeetingMoment: vi.fn() }));
const revertTopicEntry = vi.fn();
const rewriteTopicEntry = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  revertTopicEntry: (...a: unknown[]) => revertTopicEntry(...a),
  rewriteTopicEntry: (...a: unknown[]) => rewriteTopicEntry(...a),
}));

import { TimelineEntry } from '@/components/Topics/TimelineEntry';
import { entry } from './fixtures';
import type { TopicRevision } from '@/types';

const revision = (over: Partial<TopicRevision> = {}): TopicRevision => ({
  id: 'tr-1',
  entryId: 'te-1',
  decisionId: 'dec-1',
  text: 'Wave two waits for the export cap.',
  sourceMeetingId: 'm-3',
  tStart: 410,
  sourceDeletedAt: null,
  createdAt: '2026-11-09T12:00:00Z',
  ...over,
});

describe('TimelineEntry', () => {
  beforeEach(() => {
    revertTopicEntry.mockReset().mockResolvedValue(undefined);
    rewriteTopicEntry.mockReset().mockResolvedValue(undefined);
  });

  it('shows the date, meeting link, kind chip, bullets with a time link, and the Decision call-out', () => {
    render(<TimelineEntry entry={entry()} />);
    expect(screen.getByRole('button', { name: 'Product sync' })).toBeInTheDocument();
    expect(screen.getByText('Series')).toBeInTheDocument();
    expect(screen.getByText('Field mapping is done.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '1:15' })).toBeInTheDocument();
    expect(screen.getByText('Decision')).toBeInTheDocument();
    expect(screen.getByText('Migrate in two waves.')).toBeInTheDocument();
    expect(screen.queryByText('edited')).toBeNull();
  });

  it('labels mention and manual entries', () => {
    const { rerender } = render(<TimelineEntry entry={entry({ kind: 'mention' })} />);
    expect(screen.getByText('Mention')).toBeInTheDocument();
    rerender(<TimelineEntry entry={entry({ kind: 'manual', meetingId: null, meetingTitle: null, aiBody: null })} />);
    expect(screen.getByText('Yours')).toBeInTheDocument();
  });

  it('the edited marker reveals the original and Revert calls the wrapper', async () => {
    const e = entry({
      editedAt: '2026-11-04T12:00:00Z',
      body: { bullets: [{ text: 'My rewrite.', t: 75 }], decisions: [] },
    });
    render(<TimelineEntry entry={e} />);
    expect(screen.queryByText('Field mapping is done.')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'edited' }));
    expect(screen.getByText('Field mapping is done.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Revert to original' }));
    await waitFor(() => expect(revertTopicEntry).toHaveBeenCalledWith('te-1'));
  });

  it('offers Rewrite from meeting only on an unedited entry with a meeting', async () => {
    const { rerender } = render(<TimelineEntry entry={entry()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Rewrite from meeting' }));
    await waitFor(() => expect(rewriteTopicEntry).toHaveBeenCalledWith('te-1'));
    rerender(<TimelineEntry entry={entry({ editedAt: '2026-11-04T12:00:00Z' })} />);
    expect(screen.queryByRole('button', { name: 'Rewrite from meeting' })).toBeNull();
    rerender(<TimelineEntry entry={entry({ kind: 'manual', meetingId: null, meetingTitle: null, aiBody: null })} />);
    expect(screen.queryByRole('button', { name: 'Rewrite from meeting' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Edit' })).toBeInTheDocument();
  });

  it('never offers Rewrite on a Mention entry: it holds a note the owner accepted', () => {
    // An accepted note has a meeting and an AI body, but rewriting it would replace the note.
    render(<TimelineEntry entry={entry({ kind: 'mention' })} />);
    expect(screen.queryByRole('button', { name: 'Rewrite from meeting' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Edit' })).toBeInTheDocument();
  });

  it('uses the quiet outline chip for every kind (amber means armed, not authorship)', () => {
    const { rerender } = render(<TimelineEntry entry={entry()} />);
    expect(screen.getByText('Series')).toHaveAttribute('data-tone', 'outline');
    rerender(<TimelineEntry entry={entry({ kind: 'mention' })} />);
    expect(screen.getByText('Mention')).toHaveAttribute('data-tone', 'outline');
    rerender(<TimelineEntry entry={entry({ kind: 'manual', meetingId: null, meetingTitle: null, aiBody: null })} />);
    expect(screen.getByText('Yours')).toHaveAttribute('data-tone', 'outline');
  });

  it('shows Source meeting deleted in place of the link', () => {
    render(<TimelineEntry entry={entry({ meetingId: null, sourceDeletedAt: '2026-11-05T12:00:00Z' })} />);
    expect(screen.getByText('Source meeting deleted')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Product sync' })).toBeNull();
    expect(screen.queryByRole('button', { name: '1:15' })).toBeNull();
  });

  it('renders an entry with no bullets and no decisions as one muted line', () => {
    const empty = { bullets: [], decisions: [] };
    render(<TimelineEntry entry={entry({ body: empty, aiBody: empty })} />);
    const line = screen.getByText('No discussion of this topic');
    expect(line.className).toMatch(/text-muted-foreground/);
  });

  it('a No discussion row still has Edit, and Rewrite from meeting when it is a series entry', async () => {
    const empty = { bullets: [], decisions: [] };
    render(<TimelineEntry entry={entry({ body: empty, aiBody: empty })} />);
    expect(screen.getByText('No discussion of this topic')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Rewrite from meeting' }));
    await waitFor(() => expect(rewriteTopicEntry).toHaveBeenCalledWith('te-1'));
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    expect(screen.queryByText('No discussion of this topic')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Edit' })).toBeNull();
  });

  it('renders a revision under its decision', () => {
    render(<TimelineEntry entry={entry({ revisions: [revision()] })} />);
    const callout = screen.getByText('Migrate in two waves.').closest('[data-decision-id]') as HTMLElement;
    expect(callout).toHaveAttribute('data-decision-id', 'dec-1');
    expect(within(callout).getByText('Wave two waits for the export cap.')).toBeInTheDocument();
    expect(within(callout).getByText(/Nov 9/)).toBeInTheDocument();
  });

  it('renders a revision whose decision is gone at the entry foot', () => {
    const e = entry({
      editedAt: '2026-11-04T12:00:00Z',
      body: { bullets: [{ text: 'Decision removed by me.' }], decisions: [] },
      revisions: [revision()],
    });
    render(<TimelineEntry entry={e} />);
    const text = screen.getByText('Wave two waits for the export cap.');
    expect(text.closest('[data-decision-id]')).toBeNull();
    expect(text.closest('[data-entry-foot]')).not.toBeNull();
  });
});
