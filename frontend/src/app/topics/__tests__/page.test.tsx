import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  createTopic: vi.fn(),
}));
let rows: unknown[] = [];
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({ data: rows, loading: false, error: null, refresh: async () => {} }),
}));

import TopicsPage from '@/app/topics/page';

const t = (id: string, name: string, status: 'open' | 'closed', pending = 0) => ({
  id, name, description: `About ${name}`, status, lastActivity: '2026-11-03T10:00:00Z',
  pendingCount: pending, meetingCount: 1,
});

describe('/topics', () => {
  beforeEach(() => push.mockReset());

  it('shows the empty state with one New topic button and a body line', () => {
    rows = [];
    render(<TopicsPage />);
    expect(screen.getByText('No topics yet')).toBeInTheDocument();
    expect(screen.getByText('A topic keeps a running log for one project across its meetings.')).toBeInTheDocument();
    const buttons = screen.getAllByRole('button', { name: 'New topic' });
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('labels the pending count in the row meta and on the badge', () => {
    rows = [t('a', 'Atlas', 'open', 2), t('c', 'Beta', 'open', 1)];
    render(<TopicsPage />);
    expect(screen.getByText(/2 suggestions waiting/)).toBeInTheDocument();
    expect(screen.getByText(/1 suggestion waiting/)).toBeInTheDocument();
    expect(screen.getByText('2')).toHaveAttribute('aria-label', '2 suggestions waiting');
  });

  it('lists open topics and keeps Closed topics collapsed until opened', () => {
    rows = [t('a', 'Atlas', 'open', 2), t('b', 'Vendor eval', 'closed')];
    render(<TopicsPage />);
    expect(screen.getByText('Atlas')).toBeInTheDocument();
    expect(screen.getByText(/About Atlas/)).toBeInTheDocument();
    expect(screen.queryByText('Vendor eval')).toBeNull();
    const toggle = screen.getByRole('button', { name: /Closed topics/ });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(toggle);
    expect(screen.getByText('Vendor eval')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Atlas'));
    expect(push).toHaveBeenCalledWith('/topics/topic?id=a');
  });
});
