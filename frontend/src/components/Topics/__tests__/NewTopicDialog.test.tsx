import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
const createTopic = vi.fn();
vi.mock('@/lib/topics', () => ({
  createTopic: (...a: unknown[]) => createTopic(...a),
  listTopics: async () => [],
  linkTopic: vi.fn(),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import { NewTopicDialog } from '@/components/Topics/NewTopicDialog';
import { AddToTopicDialog } from '@/components/Topics/AddToTopicDialog';

const type = (label: string, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } });

describe('NewTopicDialog', () => {
  beforeEach(() => {
    push.mockReset();
    createTopic.mockReset();
    createTopic.mockResolvedValue({ id: 'topic-1' });
  });

  it('disables Create on a blank name and enables it once named', () => {
    render(<NewTopicDialog open onOpenChange={() => {}} />);
    const create = screen.getByRole('button', { name: 'Create' });
    expect(create).toBeDisabled();
    type('Name', '   ');
    expect(create).toBeDisabled();
    type('Name', 'Atlas CRM rebuild');
    expect(create).toBeEnabled();
  });

  it('creates without a link and routes to the Topic page', async () => {
    const onOpenChange = vi.fn();
    render(<NewTopicDialog open onOpenChange={onOpenChange} />);
    type('Name', ' Atlas ');
    type('Description', 'The rebuild');
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() => expect(push).toHaveBeenCalledWith('/topics/topic?id=topic-1'));
    expect(createTopic).toHaveBeenCalledWith({
      name: 'Atlas',
      description: 'The rebuild',
      linkMeetingId: undefined,
      linkSeries: false,
      backfill: false,
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('with a meetingId offers the scope choice and passes linkSeries and backfill', async () => {
    render(<NewTopicDialog open onOpenChange={() => {}} meetingId="m-1" />);
    type('Name', 'Weekly');
    expect(screen.getByRole('button', { name: 'This Meeting' })).toBeInTheDocument();
    // Series is the default, with the backfill box on.
    const backfill = screen.getByRole('checkbox', {
      name: "Also add this Series' Meetings from the last 90 days",
    });
    expect(backfill).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() =>
      expect(createTopic).toHaveBeenCalledWith(
        expect.objectContaining({ linkMeetingId: 'm-1', linkSeries: true, backfill: true }),
      ),
    );
  });

  it('This Meeting hides backfill and links only that meeting', async () => {
    render(<NewTopicDialog open onOpenChange={() => {}} meetingId="m-1" />);
    type('Name', 'One-off');
    fireEvent.click(screen.getByRole('button', { name: 'This Meeting' }));
    expect(screen.queryByRole('checkbox')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() =>
      expect(createTopic).toHaveBeenCalledWith(
        expect.objectContaining({ linkMeetingId: 'm-1', linkSeries: false, backfill: false }),
      ),
    );
  });

  it('a meeting with no series key and no title defaults to This Meeting and never sends series', async () => {
    render(<NewTopicDialog open onOpenChange={() => {}} meetingId="m-1" seriesAvailable={false} />);
    type('Name', 'One-off');
    expect(screen.getByRole('button', { name: 'Every Meeting in this Series' })).toBeDisabled();
    expect(screen.queryByRole('checkbox')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() =>
      expect(createTopic).toHaveBeenCalledWith(
        expect.objectContaining({ linkMeetingId: 'm-1', linkSeries: false, backfill: false }),
      ),
    );
  });

  it("Add to Topic…'s New Topic passes seriesAvailable through", async () => {
    render(
      <AddToTopicDialog open onOpenChange={() => {}} meetingId="m-1" linkedIds={[]} seriesAvailable={false} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'New Topic' }));
    type('Name', 'One-off');
    expect(screen.getByRole('button', { name: 'Every Meeting in this Series' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() =>
      expect(createTopic).toHaveBeenCalledWith(expect.objectContaining({ linkSeries: false })),
    );
  });
});
