import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
const updateTopic = vi.fn();
const reorderTopics = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  createTopic: vi.fn(),
  updateTopic: (...a: unknown[]) => updateTopic(...a),
  reorderTopics: (...a: unknown[]) => reorderTopics(...a),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
let rows: unknown[] = [];
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({ data: rows, loading: false, error: null, refresh: async () => {} }),
}));

import TopicsPage from '@/app/topics/page';

const t = (id: string, name: string, status: 'open' | 'closed', pending = 0, sortOrder: number | null = null) => ({
  id, name, description: `About ${name}`, status, lastActivity: '2026-11-03T10:00:00Z',
  pendingCount: pending, meetingCount: 1, sortOrder, starred: false, hideFromTodo: false,
});

describe('/topics', () => {
  beforeEach(() => {
    push.mockReset();
    updateTopic.mockReset().mockResolvedValue({});
    reorderTopics.mockReset().mockResolvedValue(undefined);
  });

  it('shows the empty state with one New Topic button and a body line', () => {
    rows = [];
    render(<TopicsPage />);
    expect(screen.getByText('No Topics yet')).toBeInTheDocument();
    expect(screen.getByText('A Topic keeps a running log for one project across its Meetings.')).toBeInTheDocument();
    const buttons = screen.getAllByRole('button', { name: 'New Topic' });
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

  it('lists open topics and keeps Archived Topics collapsed until opened', () => {
    rows = [t('a', 'Atlas', 'open', 2), t('b', 'Vendor eval', 'closed')];
    render(<TopicsPage />);
    expect(screen.getByText('Atlas')).toBeInTheDocument();
    expect(screen.getByText(/About Atlas/)).toBeInTheDocument();
    expect(screen.queryByText('Vendor eval')).toBeNull();
    const toggle = screen.getByRole('button', { name: /Archived Topics/ });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(toggle);
    expect(screen.getByText('Vendor eval')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Atlas'));
    expect(push).toHaveBeenCalledWith('/topics/topic?id=a');
  });

  it('lists topics in manual order, not by activity (specs/0090)', () => {
    rows = [t('a', 'Atlas', 'open', 0, 2), t('b', 'Beta', 'open', 0, 0), t('c', 'Cargo', 'open', 0, 1)];
    render(<TopicsPage />);
    const names = screen.getAllByText(/^(Atlas|Beta|Cargo)$/).map((n) => n.textContent);
    expect(names).toEqual(['Beta', 'Cargo', 'Atlas']);
  });

  it('reorders open topics within the shared order and saves every id', async () => {
    // Archived Beta sits between the open ones in the one shared order.
    rows = [t('a', 'Atlas', 'open', 0, 0), t('b', 'Beta', 'closed', 0, 1), t('c', 'Cargo', 'open', 0, 2)];
    render(<TopicsPage />);
    const grips = screen.getAllByRole('button', { name: 'Drag to reorder' });
    fireEvent.keyDown(grips[1], { key: 'ArrowUp' });
    expect(reorderTopics).toHaveBeenCalledWith(['c', 'b', 'a']);
    const names = screen.getAllByText(/^(Atlas|Cargo)$/).map((n) => n.textContent);
    expect(names).toEqual(['Cargo', 'Atlas']);
  });

  it('the row star toggles starred', async () => {
    rows = [t('a', 'Atlas', 'open')];
    render(<TopicsPage />);
    fireEvent.click(screen.getByRole('button', { name: 'Star Atlas' }));
    expect(updateTopic).toHaveBeenCalledWith('a', { starred: true });
    expect(screen.getByRole('button', { name: 'Unstar Atlas' })).toHaveAttribute('aria-pressed', 'true');
  });
});
