import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn(), back: vi.fn() }) }));
vi.mock('../TopicHeader', () => ({
  TopicHeader: () => <header data-testid="title-row" />,
  TopicMeta: () => <div data-testid="meta" />,
}));
vi.mock('../TopicNotesCard', () => ({ TopicNotesCard: () => <section data-testid="notes" /> }));
vi.mock('../TopicTodosCard', () => ({ TopicTodosCard: () => <section data-testid="todos" /> }));
vi.mock('../SuggestionStrip', () => ({ SuggestionStrip: () => null }));
vi.mock('../Timeline', () => ({ Timeline: () => null }));

import { detail } from './fixtures';
const data = detail();
vi.mock('@/hooks/useTopics', () => ({
  useTopic: () => ({ data, loading: false, error: null, refresh: vi.fn() }),
}));

import { TopicView } from '@/components/Topics/TopicView';

describe('TopicView', () => {
  it('pins the title row outside the scroll box; the links/description scroll with the body', () => {
    render(<TopicView id="topic-1" />);
    const scroll = screen.getByTestId('topic-scroll');
    expect(scroll).not.toContainElement(screen.getByTestId('title-row'));
    expect(scroll).toContainElement(screen.getByTestId('meta'));
    expect(scroll.className).toMatch(/overflow-y-auto/);
  });

  it('puts My notes and the action items side by side from 900px', () => {
    render(<TopicView id="topic-1" />);
    const notes = screen.getByTestId('notes');
    const todos = screen.getByTestId('todos');
    expect(notes.compareDocumentPosition(todos) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(notes.parentElement).toBe(todos.parentElement);
    expect(notes.parentElement?.className).toMatch(/min-\[900px\]:grid-cols-2/);
  });
});
