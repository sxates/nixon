import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
const updateTopic = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  updateTopic: (...a: unknown[]) => updateTopic(...a),
  draftTopicDescription: (...a: unknown[]) => draftTopicDescription(...a),
}));
const draftTopicDescription = vi.fn();

import { TopicDescription } from '@/components/Topics/TopicDescription';

describe('TopicDescription', () => {
  beforeEach(() => {
    updateTopic.mockReset().mockResolvedValue({});
    draftTopicDescription.mockReset().mockResolvedValue('A drafted description.');
  });

  it('has no separate Save button', () => {
    render(<TopicDescription topicId="topic-1" description="First." />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
  });

  it('Done saves a drafted description that was never focused, then closes', async () => {
    render(<TopicDescription topicId="topic-1" description="" />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    fireEvent.click(screen.getByRole('button', { name: 'Draft from Meetings' }));
    await screen.findByDisplayValue('A drafted description.');
    expect(updateTopic).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Done editing the description' }));
    await waitFor(() =>
      expect(updateTopic).toHaveBeenCalledWith('topic-1', { description: 'A drafted description.' }),
    );
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Description' })).toBeNull());
  });

  it('a blur save followed by Done writes once', async () => {
    render(<TopicDescription topicId="topic-1" description="First." />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    const box = screen.getByRole('textbox', { name: 'Description' });
    fireEvent.change(box, { target: { value: 'Second.' } });
    fireEvent.blur(box);
    fireEvent.click(screen.getByRole('button', { name: 'Done editing the description' }));
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Description' })).toBeNull());
    expect(updateTopic).toHaveBeenCalledTimes(1);
  });

  it('a failed save on Done keeps the field open with the text', async () => {
    updateTopic.mockRejectedValueOnce(new Error('disk full'));
    render(<TopicDescription topicId="topic-1" description="First." />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    const box = screen.getByRole('textbox', { name: 'Description' });
    fireEvent.change(box, { target: { value: 'Second.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Done editing the description' }));
    await waitFor(() => expect(updateTopic).toHaveBeenCalledTimes(1));
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveValue('Second.');
  });

  it('shows the whole description inline, with an Edit affordance and no disclosure', () => {
    const long = 'Replacing the legacy CRM.\nTwo waves: accounts first, then history.';
    render(<TopicDescription topicId="topic-1" description={long} />);
    const text = screen.getByText(/Two waves/);
    expect(text.className).not.toMatch(/line-clamp/);
    expect(screen.queryByRole('button', { expanded: false })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveValue(long);
  });

  it('takes a newer saved description when the field holds no unsaved text', () => {
    const { rerender } = render(<TopicDescription topicId="topic-1" description="First." />);
    rerender(<TopicDescription topicId="topic-1" description="Second." />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveValue('Second.');
    rerender(<TopicDescription topicId="topic-1" description="Third." />);
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveValue('Third.');
  });

  it('keeps unsaved text when a newer description arrives', () => {
    const { rerender } = render(<TopicDescription topicId="topic-1" description="First." />);
    fireEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    const box = screen.getByRole('textbox', { name: 'Description' });
    fireEvent.change(box, { target: { value: 'Mine.' } });
    rerender(<TopicDescription topicId="topic-1" description="Second." />);
    expect(box).toHaveValue('Mine.');
  });
});
