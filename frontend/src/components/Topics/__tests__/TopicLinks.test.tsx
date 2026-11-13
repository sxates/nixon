import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([
    { id: 'm-1', title: 'Product sync', createdAt: '2026-11-03T10:00:00Z' },
    { id: 'm-2', title: 'Design review', createdAt: '2026-11-04T10:00:00Z' },
    { id: 'm-3', title: 'Hiring loop', createdAt: '2026-11-05T10:00:00Z' },
  ]),
}));
const linkTopic = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  linkTopic: (...a: unknown[]) => linkTopic(...a),
}));

import { TopicLinks } from '@/components/Topics/TopicLinks';

const open = async () => {
  const onChanged = vi.fn();
  render(<TopicLinks topicId="topic-1" links={[]} meetingCount={0} onChanged={onChanged} />);
  fireEvent.click(screen.getByRole('button', { name: 'Link a series…' }));
  await screen.findByRole('checkbox', { name: /Product sync/ });
  return onChanged;
};

describe('TopicLinks — linking several series', () => {
  beforeEach(() => {
    linkTopic.mockReset().mockResolvedValue({});
  });

  it('links every checked series in one go, with the backfill choice', async () => {
    const onChanged = await open();
    const confirm = screen.getByRole('button', { name: 'Link 0 series' });
    expect(confirm).toBeDisabled();

    fireEvent.click(screen.getByRole('checkbox', { name: /Product sync/ }));
    fireEvent.click(screen.getByRole('checkbox', { name: /Hiring loop/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Link 2 series' }));

    await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
    expect(linkTopic).toHaveBeenCalledTimes(2);
    expect(linkTopic).toHaveBeenCalledWith('topic-1', 'm-1', 'series', true);
    expect(linkTopic).toHaveBeenCalledWith('topic-1', 'm-3', 'series', true);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('a second click unchecks a series', async () => {
    await open();
    const box = screen.getByRole('checkbox', { name: /Design review/ });
    fireEvent.click(box);
    fireEvent.click(box);
    expect(screen.getByRole('button', { name: 'Link 0 series' })).toBeDisabled();
  });

  it('keeps only the failed series selected and the dialog open', async () => {
    linkTopic.mockImplementation(async (_t: string, id: string) => {
      if (id === 'm-2') throw new Error('boom');
      return {};
    });
    const onChanged = await open();
    fireEvent.click(screen.getByRole('checkbox', { name: /Product sync/ }));
    fireEvent.click(screen.getByRole('checkbox', { name: /Design review/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Link 2 series' }));

    expect(await screen.findByText('We could not link one of them. Please try again.')).toBeInTheDocument();
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('checkbox', { name: /Design review/ })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('checkbox', { name: /Product sync/ })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('button', { name: 'Link 1 series' })).toBeEnabled();
  });
});
