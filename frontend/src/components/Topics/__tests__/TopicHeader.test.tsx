import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
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

import { TopicHeader } from '@/components/Topics/TopicHeader';
import { detail, entry } from './fixtures';

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

  it('shows the name, status toggle, link chip, Link a series…, and the meeting count', () => {
    render(<TopicHeader detail={linked()} />);
    expect(screen.getByRole('button', { name: /Atlas CRM rebuild/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open', pressed: true })).toBeInTheDocument();
    expect(screen.getByText('Product sync')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Link a series…' })).toBeInTheDocument();
    expect(screen.getByText('2 meetings')).toBeInTheDocument();
  });

  it('closing the topic calls updateTopic', async () => {
    render(<TopicHeader detail={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Closed', pressed: false }));
    await waitFor(() => expect(updateTopic).toHaveBeenCalledWith('topic-1', { status: 'closed' }));
  });

  it('unlinks a chip only after confirming', async () => {
    render(<TopicHeader detail={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Unlink Product sync' }));
    expect(unlinkTopic).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Unlink' }));
    await waitFor(() => expect(unlinkTopic).toHaveBeenCalledWith('topic-1', 'series', 's-1'));
  });

  it('Draft from meetings fills the description and saves only on blur', async () => {
    render(<TopicHeader detail={linked()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    fireEvent.click(screen.getByRole('button', { name: 'Draft from meetings' }));
    const box = await screen.findByDisplayValue('A drafted description.');
    expect(updateTopic).not.toHaveBeenCalled();
    fireEvent.blur(box);
    await waitFor(() =>
      expect(updateTopic).toHaveBeenCalledWith('topic-1', { description: 'A drafted description.' }),
    );
  });
});
