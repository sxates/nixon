import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn() }),
  useSearchParams: () => new URLSearchParams('id=topic-1'),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue([]) }));
vi.mock('@/lib/meetingLink', () => ({ openMeetingMoment: vi.fn() }));

import type { TopicDetail } from '@/types';
import { detail, noteSuggestion } from '@/components/Topics/__tests__/fixtures';

let current: TopicDetail = detail();
vi.mock('@/hooks/useTopics', () => ({
  useTopic: () => ({ data: current, loading: false, error: null, refresh: async () => {} }),
}));

import TopicPage from '@/app/topics/topic/page';

describe('/topics/topic', () => {
  it('lays out notes, action items, and the timeline; no strip without suggestions', () => {
    current = detail();
    render(<TopicPage />);
    expect(screen.getByText('My notes')).toBeInTheDocument();
    expect(screen.getByText('My open action items')).toBeInTheDocument();
    expect(screen.getByText('Timeline')).toBeInTheDocument();
    expect(screen.queryByText('Suggested notes')).toBeNull();
  });

  it('shows the strip when suggestions are pending', () => {
    current = detail({ suggestions: [noteSuggestion()] });
    render(<TopicPage />);
    expect(screen.getByText('Suggested notes')).toBeInTheDocument();
  });
});
