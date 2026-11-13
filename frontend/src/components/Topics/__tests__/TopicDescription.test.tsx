import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
const updateTopic = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  updateTopic: (...a: unknown[]) => updateTopic(...a),
  draftTopicDescription: vi.fn(),
}));

import { TopicDescription } from '@/components/Topics/TopicDescription';

describe('TopicDescription', () => {
  beforeEach(() => updateTopic.mockReset().mockResolvedValue({}));

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
