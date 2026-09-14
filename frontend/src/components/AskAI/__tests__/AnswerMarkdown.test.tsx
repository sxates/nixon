import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

// specs/0035 — [M#] citation chips: `sources[i]` ↔ `[M{i+1}]`. In-range markers
// become link chips to /meeting-details; out-of-range/malformed markers render
// as plain text (defense in depth — Rust strips them already).

const pushMock = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock }),
}));

import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import type { SourceMeeting } from '@/lib/ask-ai';

const SOURCES: SourceMeeting[] = [
  { meetingId: 'meet-1', title: 'Pricing sync', createdAt: '2026-06-20T10:00:00Z', cited: true },
  { meetingId: 'meet-2', title: 'Launch review', createdAt: '2026-06-27T10:00:00Z', cited: true },
];

beforeEach(() => {
  pushMock.mockClear();
});

describe('AnswerMarkdown [M#] chip rendering (specs/0035)', () => {
  it('renders in-range markers as chips labelled with the meeting title', () => {
    render(
      <AnswerMarkdown
        markdown="We agreed to raise the price [M1] and ship in July [M2]."
        sources={SOURCES}
      />,
    );

    expect(screen.getByRole('button', { name: 'Pricing sync' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Launch review' })).toBeTruthy();
    // The markers themselves are replaced, not shown.
    expect(screen.queryByText(/\[M1\]/)).toBeNull();
  });

  it('chips link to the right meeting', () => {
    render(<AnswerMarkdown markdown="Decided [M2]." sources={SOURCES} />);

    fireEvent.click(screen.getByRole('button', { name: 'Launch review' }));
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=meet-2');
  });

  it('out-of-range markers stay as plain text, never a chip', () => {
    render(<AnswerMarkdown markdown="Unknown claim [M9] here." sources={SOURCES} />);

    expect(screen.getByText(/Unknown claim \[M9\] here\./)).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('[M0] is out of range too (numbering is 1-based)', () => {
    render(<AnswerMarkdown markdown="Nothing at [M0]." sources={SOURCES} />);

    expect(screen.getByText(/Nothing at \[M0\]\./)).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('text without markers renders as plain markdown (no chips)', () => {
    render(
      <AnswerMarkdown
        markdown={'## Decisions\n- **Raise** the price\n- Ship in *July*'}
        sources={SOURCES}
      />,
    );

    expect(screen.getByRole('heading', { name: 'Decisions' })).toBeTruthy();
    expect(screen.getAllByRole('listitem')).toHaveLength(2);
    expect(screen.getByText('Raise').tagName).toBe('STRONG');
    expect(screen.getByText('July').tagName).toBe('EM');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('a chip labels with M# when the source title is empty', () => {
    render(
      <AnswerMarkdown
        markdown="See [M1]."
        sources={[{ meetingId: 'meet-1', title: '  ', createdAt: '2026-06-20T10:00:00Z', cited: true }]}
      />,
    );

    expect(screen.getByRole('button', { name: 'M1' })).toBeTruthy();
  });

  it('mixed line: chip embedded between emphasized text segments', () => {
    render(
      <AnswerMarkdown markdown="**Decision:** raise prices [M1] next quarter." sources={SOURCES} />,
    );

    expect(screen.getByText('Decision:').tagName).toBe('STRONG');
    expect(screen.getByRole('button', { name: 'Pricing sync' })).toBeTruthy();
    expect(screen.getByText(/next quarter\./)).toBeTruthy();
  });
});
