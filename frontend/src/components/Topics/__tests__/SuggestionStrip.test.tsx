import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
const toastError = vi.fn();
vi.mock('sonner', () => ({ toast: { error: (...a: unknown[]) => toastError(...a), success: vi.fn() } }));
const openMeetingMoment = vi.fn();
vi.mock('@/lib/meetingLink', () => ({
  openMeetingMoment: (...a: unknown[]) => openMeetingMoment(...a),
}));
const resolveTopicSuggestion = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  resolveTopicSuggestion: (...a: unknown[]) => resolveTopicSuggestion(...a),
}));

import { SuggestionStrip } from '@/components/Topics/SuggestionStrip';
import { noteSuggestion, revisionSuggestion } from './fixtures';

describe('SuggestionStrip', () => {
  beforeEach(() => {
    resolveTopicSuggestion.mockReset().mockResolvedValue(undefined);
    toastError.mockReset();
    openMeetingMoment.mockReset();
  });

  it('renders nothing when there are no suggestions', () => {
    const { container } = render(<SuggestionStrip suggestions={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('shows a note row with its meeting, time link, and actions', () => {
    render(<SuggestionStrip suggestions={[noteSuggestion()]} />);
    expect(screen.getByText('Suggested notes')).toBeInTheDocument();
    expect(screen.getByText('Support weekly')).toBeInTheDocument();
    expect(screen.getByText('Support wants a read-only view during the pilot.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '3:40' }));
    expect(openMeetingMoment).toHaveBeenCalledWith(expect.anything(), 'm-2', 220);
    expect(screen.getByRole('button', { name: 'Add to topic' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Dismiss' })).toBeInTheDocument();
  });

  it('Add to topic accepts the suggestion; Dismiss rejects it', async () => {
    render(<SuggestionStrip suggestions={[noteSuggestion()]} />);
    fireEvent.click(screen.getByRole('button', { name: 'Add to topic' }));
    await waitFor(() => expect(resolveTopicSuggestion).toHaveBeenCalledWith('ts-1', true));
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(resolveTopicSuggestion).toHaveBeenCalledWith('ts-1', false));
  });

  it('words a revision row exactly, with Attach note', async () => {
    render(<SuggestionStrip suggestions={[revisionSuggestion()]} />);
    expect(
      screen.getByText('May change the Nov 3 decision: Migrate in two waves.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Start with the largest region instead.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Attach note' }));
    await waitFor(() => expect(resolveTopicSuggestion).toHaveBeenCalledWith('ts-2', true));
  });

  it('words a revision whose target date or decision text is gone without a broken tail', () => {
    render(
      <SuggestionStrip
        suggestions={[
          revisionSuggestion({ id: 'ts-a', targetOccurredAt: null }),
          revisionSuggestion({ id: 'ts-b', targetDecisionText: null }),
          revisionSuggestion({ id: 'ts-c', targetOccurredAt: null, targetDecisionText: null }),
        ]}
      />,
    );
    expect(screen.getByText('May change an earlier decision: Migrate in two waves.')).toBeInTheDocument();
    expect(screen.getByText('May change the Nov 3 decision.')).toBeInTheDocument();
    expect(screen.getByText('May change an earlier decision.')).toBeInTheDocument();
  });

  it('toasts a resolve error', async () => {
    resolveTopicSuggestion.mockRejectedValue('That entry was deleted.');
    render(<SuggestionStrip suggestions={[revisionSuggestion()]} />);
    fireEvent.click(screen.getByRole('button', { name: 'Attach note' }));
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(JSON.stringify(toastError.mock.calls[0])).toContain('That entry was deleted.');
  });
});
