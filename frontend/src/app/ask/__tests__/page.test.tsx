import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Owner feedback 2026-09-21, four items on this page:
//  - the Ask box is rearranged (input full width on its own line, Ask/Save left and the
//    filters right, in one row);
//  - "All time" is too broad a default — 7d instead;
//  - the "who" filter's label lied by omission: it selects MEETINGS someone was in, not
//    their transcript lines;
//  - a history entry expands in place instead of taking over the answer card at the top.
//
// The page had no tests at all before this, which is how the history card ended up meaning
// two different things with a banner to tell them apart.

const { invoke, listeners, clipboard } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
  clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(event, cb);
    return () => listeners.delete(event);
  },
}));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn() }),
  useSearchParams: () => new URLSearchParams(''),
}));

import AskPage from '@/app/ask/page';

const HISTORY = [
  {
    id: 'aah-1',
    question: 'What did we decide about rev C?',
    scopeJson: '{}',
    answerMarkdown: 'We ship rev C in October [M1], pending the thermal retest [M2].',
    sourcesJson: JSON.stringify([
      { meetingId: 'm1', title: 'Product sync', createdAt: '2026-09-20T14:00:00.000Z' },
      { meetingId: 'm2', title: 'Hardware review', createdAt: '2026-09-18T14:00:00.000Z' },
    ]),
    provider: 'ollama',
    model: 'gemma',
    createdAt: '2026-09-20T15:00:00.000Z',
  },
  {
    id: 'aah-2',
    question: 'Who owns the thermal retest?',
    scopeJson: '{}',
    answerMarkdown: 'Maya owns it [M1].',
    sourcesJson: JSON.stringify([
      { meetingId: 'm1', title: 'Product sync', createdAt: '2026-09-20T14:00:00.000Z' },
    ]),
    provider: 'ollama',
    model: 'gemma',
    createdAt: '2026-09-18T15:00:00.000Z',
  },
];

const PEOPLE = [{ id: 'p1', displayName: 'Maya Okafor', email: 'maya@example.com' }];

beforeEach(() => {
  invoke.mockReset();
  listeners.clear();
  clipboard.writeText.mockClear();
  Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true });
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_list_ask_ai_history') return HISTORY;
    if (cmd === 'api_list_saved_questions') return [];
    if (cmd === 'api_list_people') return PEOPLE;
    if (cmd === 'api_ask_ai_run') return 'run-1';
    return null;
  });
});

/** The History section, opened. */
async function openHistory() {
  const toggle = await screen.findByRole('button', { name: /History/ });
  await userEvent.click(toggle);
  return toggle;
}

describe('Ask AI — scope defaults and labels', () => {
  it('defaults the date scope to 7d, not All time', async () => {
    render(<AskPage />);
    const group = await screen.findByRole('group', { name: 'Filter by date' });
    expect(within(group).getByRole('button', { name: '7d' }).getAttribute('aria-pressed')).toBe(
      'true',
    );
    expect(
      within(group).getByRole('button', { name: 'All time' }).getAttribute('aria-pressed'),
    ).toBe('false');
  });

  it('runs with a bounded date scope by default', async () => {
    render(<AskPage />);
    await userEvent.type(await screen.findByLabelText('Question'), 'what happened');
    await userEvent.click(screen.getByRole('button', { name: /^Ask$/ }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_ask_ai_run', expect.anything()));
    const scope = invoke.mock.calls.find((c) => c[0] === 'api_ask_ai_run')![1].scope;
    // 7d means a lower bound exists — the old 'all' default sent neither bound.
    expect(scope.dateFrom).toBeTruthy();
  });

  it('says the person filter selects meetings, not lines', async () => {
    render(<AskPage />);
    const trigger = await screen.findByRole('button', {
      name: /Filter by the meetings someone was in/i,
    });
    expect(trigger.textContent).toContain('Any meeting');
    // The old label was the bare word "Anyone", which read as "any speaker".
    expect(trigger.textContent).not.toContain('Anyone');
    expect(trigger.getAttribute('title')).toMatch(/does not narrow the answer to their lines/i);

    await userEvent.click(trigger);
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Meetings with Maya Okafor' }));
    expect(
      screen.getByRole('button', { name: /Filter by the meetings someone was in/i }).textContent,
    ).toContain('Meetings with Maya Okafor');
  });
});

describe('Ask AI — history expands in place', () => {
  it('opens the answer under its own row, leaving the top card alone', async () => {
    render(<AskPage />);
    await openHistory();

    const row = await screen.findByRole('button', { name: /What did we decide about rev C\?/ });
    expect(screen.queryByText(/We ship rev C in October/)).not.toBeInTheDocument();

    await userEvent.click(row);
    // The answer is on screen…
    expect(await screen.findByText(/We ship rev C in October/)).toBeInTheDocument();
    // …and the question box was NOT hijacked into showing that past question.
    expect((screen.getByLabelText('Question') as HTMLInputElement).value).toBe('');
    // The banner that existed only to disambiguate the shared card is gone.
    expect(screen.queryByText(/Saved answer from your history/)).not.toBeInTheDocument();
  });

  it('is an accordion — opening one closes the other', async () => {
    render(<AskPage />);
    await openHistory();

    await userEvent.click(
      await screen.findByRole('button', { name: /What did we decide about rev C\?/ }),
    );
    expect(await screen.findByText(/We ship rev C in October/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /Who owns the thermal retest\?/ }));
    expect(await screen.findByText(/Maya owns it/)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByText(/We ship rev C in October/)).not.toBeInTheDocument(),
    );
  });

  it('clicking the same row again collapses it', async () => {
    render(<AskPage />);
    await openHistory();

    const row = await screen.findByRole('button', { name: /What did we decide about rev C\?/ });
    await userEvent.click(row);
    expect(await screen.findByText(/We ship rev C in October/)).toBeInTheDocument();
    await userEvent.click(row);
    await waitFor(() =>
      expect(screen.queryByText(/We ship rev C in October/)).not.toBeInTheDocument(),
    );
  });

  it('Ask again re-runs the question through the live card', async () => {
    render(<AskPage />);
    await openHistory();
    await userEvent.click(
      await screen.findByRole('button', { name: /What did we decide about rev C\?/ }),
    );
    await userEvent.click(await screen.findByRole('button', { name: 'Ask again' }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        'api_ask_ai_run',
        expect.objectContaining({ question: 'What did we decide about rev C?' }),
      ),
    );
    // The row collapsed, so there is exactly one answer surface in play.
    await waitFor(() =>
      expect(screen.queryByText(/We ship rev C in October/)).not.toBeInTheDocument(),
    );
  });
});

describe('Ask AI — copying an answer', () => {
  it('copies the prose without the citation markers', async () => {
    render(<AskPage />);
    await openHistory();
    await userEvent.click(
      await screen.findByRole('button', { name: /What did we decide about rev C\?/ }),
    );

    await userEvent.click(await screen.findByRole('button', { name: 'Copy answer' }));
    expect(clipboard.writeText).toHaveBeenCalledWith(
      'We ship rev C in October, pending the thermal retest.',
    );
  });
});
