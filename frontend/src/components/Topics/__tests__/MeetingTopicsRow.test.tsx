import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
const topicsForMeeting = vi.fn();
const listTopics = vi.fn();
const linkTopic = vi.fn();
vi.mock('@/lib/topics', () => ({
  topicsForMeeting: (...a: unknown[]) => topicsForMeeting(...a),
  listTopics: (...a: unknown[]) => listTopics(...a),
  linkTopic: (...a: unknown[]) => linkTopic(...a),
  createTopic: vi.fn(),
}));
let updated: ((e: { payload: { topic_id: string | null } }) => void) | null = null;
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (_n: string, cb: typeof updated) => {
    updated = cb;
    return () => {};
  },
}));
const toastError = vi.fn();
vi.mock('sonner', () => ({ toast: { error: (...a: unknown[]) => toastError(...a), success: vi.fn() } }));

import { MeetingTopicsRow } from '@/components/Topics/MeetingTopicsRow';

const topic = (id: string, name: string) => ({ id, name, description: '', status: 'open' });

describe('MeetingTopicsRow', () => {
  beforeEach(() => {
    push.mockReset();
    linkTopic.mockReset();
    toastError.mockReset();
    topicsForMeeting.mockResolvedValue([topic('t1', 'Atlas CRM rebuild')]);
    listTopics.mockResolvedValue([
      { ...topic('t1', 'Atlas CRM rebuild') },
      { ...topic('t2', 'Pricing review') },
    ]);
    linkTopic.mockResolvedValue({});
  });

  it('renders a chip per topic that routes to the topic page', async () => {
    render(<MeetingTopicsRow meetingId="m-1" title="Weekly" seriesKey={null} />);
    expect(screen.getByText('Topics')).toBeInTheDocument();
    fireEvent.click(await screen.findByRole('button', { name: 'Atlas CRM rebuild' }));
    expect(push).toHaveBeenCalledWith('/topics/topic?id=t1');
  });

  it('refreshes on topics-updated', async () => {
    render(<MeetingTopicsRow meetingId="m-1" title="Weekly" seriesKey={null} />);
    await screen.findByRole('button', { name: 'Atlas CRM rebuild' });
    topicsForMeeting.mockResolvedValue([topic('t1', 'Atlas CRM rebuild'), topic('t2', 'Pricing review')]);
    updated?.({ payload: { topic_id: null } });
    expect(await screen.findByRole('button', { name: 'Pricing review' })).toBeInTheDocument();
  });

  it('dialog passes scope and backfill to linkTopic', async () => {
    render(<MeetingTopicsRow meetingId="m-1" title="Weekly" seriesKey="ser-1" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add to topic…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Pricing review' }));
    fireEvent.click(screen.getByRole('button', { name: 'Every meeting in this series' }));
    fireEvent.click(
      screen.getByRole('checkbox', { name: "Also add this series' meetings from the last 90 days" }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(linkTopic).toHaveBeenCalledWith('t2', 'm-1', 'series', true));
  });

  it('hides backfill for This meeting and links with scope meeting', async () => {
    render(<MeetingTopicsRow meetingId="m-1" title="Weekly" seriesKey="ser-1" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add to topic…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Pricing review' }));
    expect(screen.queryByRole('checkbox')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(linkTopic).toHaveBeenCalledWith('t2', 'm-1', 'meeting', false));
  });

  it('disables the series option with neither a series key nor a title', async () => {
    render(<MeetingTopicsRow meetingId="m-1" title="  " seriesKey={null} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add to topic…' }));
    await screen.findByRole('button', { name: 'Pricing review' });
    expect(screen.getByRole('button', { name: 'Every meeting in this series' })).toBeDisabled();
  });

  it('toasts a link failure', async () => {
    linkTopic.mockRejectedValue("This meeting isn't part of a series.");
    render(<MeetingTopicsRow meetingId="m-1" title="Weekly" seriesKey={null} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add to topic…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Pricing review' }));
    fireEvent.click(screen.getByRole('button', { name: 'Every meeting in this series' }));
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(toastError).toHaveBeenCalled());
  });
});
