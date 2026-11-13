import React from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
const setTopicNotes = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  setTopicNotes: (...a: unknown[]) => setTopicNotes(...a),
}));

import { TopicNotesCard } from '@/components/Topics/TopicNotesCard';
import { topic } from './fixtures';

describe('TopicNotesCard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setTopicNotes.mockReset().mockResolvedValue(undefined);
  });
  afterEach(() => vi.useRealTimers());

  it('shows the title and hint and seeds from saved markdown', () => {
    render(<TopicNotesCard topic={topic({ notesMarkdown: 'Check the export limits.' })} />);
    expect(screen.getByText('My notes')).toBeInTheDocument();
    expect(screen.getByText('Yours. AI never rewrites this.')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'My notes' })).toHaveTextContent('Check the export limits.');
  });

  it('autosaves 1000 ms after typing stops, markdown plus HTML', async () => {
    render(<TopicNotesCard topic={topic()} />);
    const box = screen.getByRole('textbox', { name: 'My notes' });
    box.innerHTML = '<p>First</p>';
    fireEvent.input(box);
    act(() => void vi.advanceTimersByTime(600));
    box.innerHTML = '<p>First line</p>';
    fireEvent.input(box);
    act(() => void vi.advanceTimersByTime(999));
    expect(setTopicNotes).not.toHaveBeenCalled();
    await act(async () => void vi.advanceTimersByTime(1));
    expect(setTopicNotes).toHaveBeenCalledTimes(1);
    expect(setTopicNotes).toHaveBeenCalledWith('topic-1', 'First line', '<p>First line</p>');
  });

  it('flushes a pending save on unmount', () => {
    const { unmount } = render(<TopicNotesCard topic={topic()} />);
    const box = screen.getByRole('textbox', { name: 'My notes' });
    box.innerHTML = '<p>Last words</p>';
    fireEvent.input(box);
    unmount();
    expect(setTopicNotes).toHaveBeenCalledWith('topic-1', 'Last words', '<p>Last words</p>');
  });

  it('has no AI affordance', () => {
    const { container } = render(<TopicNotesCard topic={topic()} />);
    expect(container.querySelector('.lucide-sparkles')).toBeNull();
  });
});
