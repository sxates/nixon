import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
let rows: unknown[] = [];
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({ data: rows, loading: false, error: null, refresh: async () => {} }),
}));

import { TopicsNavList } from '@/components/Sidebar/TopicsNavList';

const mk = (n: number) =>
  Array.from({ length: n }, (_, i) => ({
    id: `t${i}`, name: `Topic ${i}`, description: '', status: 'open',
    lastActivity: `2026-11-${String(i + 1).padStart(2, '0')}T10:00:00Z`,
    pendingCount: i === 0 ? 3 : 0, meetingCount: 0,
  }));

describe('TopicsNavList', () => {
  beforeEach(() => push.mockReset());

  it('renders nothing when collapsed or empty', () => {
    rows = mk(2);
    const { container, rerender } = render(<TopicsNavList collapsed />);
    expect(container).toBeEmptyDOMElement();
    rows = [];
    rerender(<TopicsNavList collapsed={false} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('caps at 6, most recent first, with a +N more row to /topics', () => {
    rows = mk(9);
    render(<TopicsNavList collapsed={false} />);
    const names = screen.getAllByRole('button').map((b) => b.getAttribute('aria-label'));
    expect(names.slice(0, 6)).toEqual([8, 7, 6, 5, 4, 3].map((i) => `Topic ${i}`));
    // No count: the name alone.
    fireEvent.click(screen.getByRole('button', { name: '+3 more' }));
    expect(push).toHaveBeenCalledWith('/topics');
  });

  it('shows a pending badge and opens the topic', () => {
    rows = mk(1);
    render(<TopicsNavList collapsed={false} />);
    expect(screen.getByText('3')).toHaveAttribute('aria-label', '3 suggestions waiting');
    // The row's accessible name carries the count, not just the topic name.
    fireEvent.click(screen.getByRole('button', { name: 'Topic 0, 3 suggestions waiting' }));
    expect(push).toHaveBeenCalledWith('/topics/topic?id=t0');
    expect(screen.queryByRole('button', { name: /more/ })).toBeNull();
  });
});
