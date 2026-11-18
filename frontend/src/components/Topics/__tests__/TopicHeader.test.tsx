import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push, back: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue([]) }));
const updateTopic = vi.fn();
const draftTopicDescription = vi.fn();
const unlinkTopic = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  updateTopic: (...a: unknown[]) => updateTopic(...a),
  draftTopicDescription: (...a: unknown[]) => draftTopicDescription(...a),
  unlinkTopic: (...a: unknown[]) => unlinkTopic(...a),
}));

import { TopicHeader, TopicMeta, TopicSeriesLinks } from '@/components/Topics/TopicHeader';
import type { TopicDetail } from '@/types';
import { detail, entry } from './fixtures';

const Both = ({ d }: { d: TopicDetail }) => (
  <>
    <TopicHeader detail={d} />
    <TopicMeta detail={d} />
    <TopicSeriesLinks detail={d} />
  </>
);

const linked = () =>
  detail({
    links: [{ topicId: 'topic-1', kind: 'series', ref: 's-1', title: 'product sync', label: 'Product sync' }],
    entries: [entry(), entry({ id: 'te-2', meetingId: 'm-2' }), entry({ id: 'te-3', meetingId: 'm-2' })],
  });

describe('TopicHeader', () => {
  beforeEach(() => {
    updateTopic.mockReset().mockResolvedValue({});
    draftTopicDescription.mockReset().mockResolvedValue('A drafted description.');
    unlinkTopic.mockReset().mockResolvedValue(undefined);
  });

  it('shows the name, link chip, Link a Series…, and the Meeting count; no status toggle', () => {
    render(<Both d={linked()} />);
    expect(screen.getByRole('button', { name: /Atlas CRM rebuild/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open' })).toBeNull();
    expect(screen.queryByText('Archived')).toBeNull();
    expect(screen.getByText('Product sync')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Link a Series…' })).toBeInTheDocument();
    expect(screen.getByText('2 Meetings')).toBeInTheDocument();
  });

  it('Archive Topic in the menu stores the closed status', async () => {
    render(<TopicHeader detail={linked()} />);
    await userEvent.click(screen.getByRole('button', { name: 'Topic options' }));
    expect(screen.queryByRole('menuitem', { name: 'Unarchive Topic' })).toBeNull();
    await userEvent.click(screen.getByRole('menuitem', { name: 'Archive Topic' }));
    await waitFor(() => expect(updateTopic).toHaveBeenCalledWith('topic-1', { status: 'closed' }));
  });

  it('an archived topic says so and offers Unarchive Topic', async () => {
    const d = linked();
    render(<TopicHeader detail={{ ...d, topic: { ...d.topic, status: 'closed' } }} />);
    expect(screen.getByText('Archived')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Topic options' }));
    expect(screen.queryByRole('menuitem', { name: 'Archive Topic' })).toBeNull();
    await userEvent.click(screen.getByRole('menuitem', { name: 'Unarchive Topic' }));
    await waitFor(() => expect(updateTopic).toHaveBeenCalledWith('topic-1', { status: 'open' }));
  });

  it('the star toggles starred, optimistically (specs/0090)', async () => {
    render(<TopicHeader detail={linked()} />);
    const star = screen.getByRole('button', { name: 'Star Topic' });
    expect(star).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(star);
    expect(screen.getByRole('button', { name: 'Unstar Topic' })).toHaveAttribute('aria-pressed', 'true');
    await waitFor(() => expect(updateTopic).toHaveBeenCalledWith('topic-1', { starred: true }));
  });

  it('a starred Topic shows a gold star (the star token, not brand)', () => {
    render(<TopicHeader detail={detail({ topic: { ...linked().topic, starred: true } })} />);
    const star = screen.getByRole('button', { name: 'Unstar Topic' });
    // On the glyph: PageHeader recolours ghost buttons, so a class on the button loses.
    const glyph = star.querySelector('svg')!;
    expect(glyph.getAttribute('class')).toContain('text-star');
    expect(star.className).not.toContain('text-brand');
  });

  it('a failed star reverts', async () => {
    updateTopic.mockRejectedValueOnce(new Error('nope'));
    render(<TopicHeader detail={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Star Topic' }));
    expect(await screen.findByRole('button', { name: 'Star Topic' })).toHaveAttribute('aria-pressed', 'false');
  });

  it('the menu no longer carries the To-Do switch (it lives on the action items card)', async () => {
    render(<TopicHeader detail={linked()} />);
    await userEvent.click(screen.getByRole('button', { name: 'Topic options' }));
    expect(screen.queryByRole('menuitemcheckbox')).toBeNull();
    expect(screen.queryByText(/To-Do list|To-Do/)).toBeNull();
  });

  it('unlinks a chip only after confirming', async () => {
    render(<Both d={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Unlink Product sync' }));
    expect(unlinkTopic).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Unlink' }));
    await waitFor(() => expect(unlinkTopic).toHaveBeenCalledWith('topic-1', 'series', 's-1'));
  });

  it('Draft from Meetings fills the description and saves only on blur', async () => {
    render(<Both d={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    fireEvent.click(screen.getByRole('button', { name: 'Draft from Meetings' }));
    const box = await screen.findByDisplayValue('A drafted description.');
    expect(updateTopic).not.toHaveBeenCalled();
    fireEvent.blur(box);
    await waitFor(() =>
      expect(updateTopic).toHaveBeenCalledWith('topic-1', { description: 'A drafted description.' }),
    );
  });
});
