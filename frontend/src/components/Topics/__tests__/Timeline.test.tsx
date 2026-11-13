import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@/lib/meetingLink', () => ({ openMeetingMoment: vi.fn() }));

import { Timeline } from '@/components/Topics/Timeline';
import { entry } from './fixtures';

describe('Timeline', () => {
  it('shows the empty state and the AI marker on its header', () => {
    const { container } = render(<Timeline topicId="topic-1" entries={[]} />);
    expect(screen.getByText('Timeline')).toBeInTheDocument();
    expect(screen.getByText('Nothing logged yet')).toBeInTheDocument();
    // Positive control for the notes card's "no Sparkles" check.
    expect(container.querySelector('.lucide-sparkles')).not.toBeNull();
  });

  it('Add entry opens the editor', () => {
    render(<Timeline topicId="topic-1" entries={[entry()]} />);
    expect(screen.queryByText('Nothing logged yet')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add entry' }));
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
  });
});
