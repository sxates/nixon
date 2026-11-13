import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const push = vi.fn();
let pathname = '/';
let search = '';
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push }),
  usePathname: () => pathname,
  useSearchParams: () => new URLSearchParams(search),
}));
let rows: unknown[] = [];
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({ data: rows, loading: false, error: null, refresh: async () => {} }),
}));

import { TopicsNav, TopicsNavList } from '@/components/Sidebar/TopicsNavList';
import type { TopicSummary } from '@/types';

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
    const { container, rerender } = render(<TopicsNavList collapsed topics={rows as TopicSummary[]} />);
    expect(container).toBeEmptyDOMElement();
    rows = [];
    rerender(<TopicsNavList collapsed={false} topics={rows as TopicSummary[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('caps at 6, most recent first, with a +N more row to /topics', () => {
    rows = mk(9);
    render(<TopicsNavList collapsed={false} topics={rows as TopicSummary[]} />);
    const names = screen.getAllByRole('button').map((b) => b.getAttribute('aria-label'));
    expect(names.slice(0, 6)).toEqual([8, 7, 6, 5, 4, 3].map((i) => `Topic ${i}`));
    // No count: the name alone.
    fireEvent.click(screen.getByRole('button', { name: '+3 more' }));
    expect(push).toHaveBeenCalledWith('/topics');
  });

  it('shows a pending badge and opens the topic', () => {
    rows = mk(1);
    render(<TopicsNavList collapsed={false} topics={rows as TopicSummary[]} />);
    expect(screen.getByText('3')).toHaveAttribute('aria-label', '3 suggestions waiting');
    // The row's accessible name carries the count, not just the topic name.
    fireEvent.click(screen.getByRole('button', { name: 'Topic 0, 3 suggestions waiting' }));
    expect(push).toHaveBeenCalledWith('/topics/topic?id=t0');
    expect(screen.queryByRole('button', { name: /more/ })).toBeNull();
  });

  it("lights the open topic's row, not the Topics row", () => {
    rows = mk(3);
    pathname = '/topics/topic';
    search = 'id=t1';
    render(<TopicsNav collapsed={false} />);
    expect(screen.getByRole('button', { name: 'Topic 1' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Topics' })).not.toHaveAttribute('aria-current');
  });

  it('lights the Topics row when the open topic is not listed, or on the index', () => {
    rows = mk(3);
    pathname = '/topics/topic';
    search = 'id=closed-one';
    const { unmount } = render(<TopicsNav collapsed={false} />);
    expect(screen.getByRole('button', { name: 'Topics' })).toHaveAttribute('aria-current', 'page');
    unmount();
    pathname = '/topics';
    search = '';
    render(<TopicsNav collapsed={false} />);
    expect(screen.getByRole('button', { name: 'Topics' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Topic 1' })).not.toHaveAttribute('aria-current');
  });
});
