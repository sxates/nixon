import React from 'react';
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// specs/0033 — the ⌘K palette's full-text search tier. Locks:
// (a) sentinel-delimited snippets render as <mark> spans, never as raw HTML;
// (b) the empty-query palette (Actions + recent meetings by title) is unchanged —
//     no backend search call, no third group;
// (c) a ≥2-char query debounces one api_search_meetings call and renders the
//     "In transcripts, summaries & notes" tier with deep-link navigation;
// (d) a backend failure falls back to the title tier without breaking the palette;
// (e) while backend search is active, the manual filtering pass reuses cmdk's
//     defaultFilter over the same value+keywords surface — matches stay stable
//     across the 2-char threshold and pasted meeting ids keep matching;
// (f) the meetings list is refetched on EVERY open (no session-long latch, and a
//     failed fetch retries instead of latching an empty list);
// (g) specs/0035: the always-visible Ask-AI row sits in its own LAST group — the
//     top search hit keeps cmdk's first-item auto-selection (type-then-Enter
//     opens the result) — and the empty state is scoped ("No matching meetings")
//     so it never contradicts the visible Ask row.
// The deep-link lifecycle itself (?segment= consume-then-clear, pagination pump)
// is covered in useSegmentDeepLink.test.ts + VirtualizedTranscriptView.deeplink.test.tsx.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const pushMock = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock }),
}));

import CommandPalette, {
  openCommandPalette,
  renderSnippet,
  type MeetingSearchHit,
} from '@/components/CommandPalette';

const MEETINGS = [
  { id: 'm1', title: 'Design sync', createdAt: '2026-06-24T10:00:00Z' },
  { id: 'm2', title: 'Kickoff planning', createdAt: '2026-06-25T10:00:00Z' },
];

const HITS: MeetingSearchHit[] = [
  {
    meetingId: 'm2',
    title: 'Kickoff planning',
    createdAt: '2026-06-25T10:00:00Z',
    source: 'transcript',
    snippet: 'we said the \u0001kickoff\u0002 is next week',
    transcriptId: 'seg-42',
    rank: -3.2,
  },
  {
    meetingId: 'm1',
    title: 'Design sync',
    createdAt: '2026-06-24T10:00:00Z',
    source: 'notes',
    snippet: '\u0001kickoff\u0002 checklist',
    transcriptId: null,
    rank: -1.1,
  },
];

function setupInvoke({ searchError = false } = {}) {
  invokeMock.mockImplementation((command: unknown) => {
    if (command === 'api_get_meetings') return Promise.resolve(MEETINGS);
    if (command === 'api_search_meetings') {
      return searchError ? Promise.reject(new Error('fts down')) : Promise.resolve(HITS);
    }
    return Promise.resolve(null);
  });
}

async function openPalette() {
  render(<CommandPalette />);
  act(() => {
    openCommandPalette();
  });
  // Meetings load async when the palette opens.
  await screen.findByText('Design sync');
}

function searchCalls() {
  return invokeMock.mock.calls.filter(([cmd]) => cmd === 'api_search_meetings');
}

function typeQuery(value: string) {
  const input = screen.getByPlaceholderText('Search meetings or run a command...');
  fireEvent.change(input, { target: { value } });
}

/** Toggle the palette via its ⌘K window listener (open ↔ close). */
function togglePalette() {
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
}

beforeEach(() => {
  invokeMock.mockReset();
  pushMock.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('renderSnippet (sentinel → <mark>)', () => {
  it('renders text between \\u0001/\\u0002 sentinels as <mark>, the rest as plain text', () => {
    const { container } = render(
      <p>{renderSnippet('we said the \u0001kickoff\u0002 is next week')}</p>,
    );
    const marks = container.querySelectorAll('mark');
    expect(marks).toHaveLength(1);
    expect(marks[0]).toHaveTextContent('kickoff');
    expect(container.textContent).toBe('we said the kickoff is next week');
  });

  it('handles multiple matches in one snippet', () => {
    const { container } = render(
      <p>{renderSnippet('\u0001alpha\u0002 then \u0001beta\u0002')}</p>,
    );
    const marks = Array.from(container.querySelectorAll('mark')).map((m) => m.textContent);
    expect(marks).toEqual(['alpha', 'beta']);
    expect(container.textContent).toBe('alpha then beta');
  });

  it('never interprets snippet content as HTML', () => {
    const { container } = render(
      <p>{renderSnippet('<b onmouseover="x()">bold</b> \u0001<i>hit</i>\u0002')}</p>,
    );
    expect(container.querySelector('b')).toBeNull();
    expect(container.querySelector('i')).toBeNull();
    expect(container.textContent).toBe('<b onmouseover="x()">bold</b> <i>hit</i>');
    expect(container.querySelector('mark')).toHaveTextContent('<i>hit</i>');
  });

  it('strips stray/unpaired sentinels instead of rendering control characters', () => {
    const { container } = render(<p>{renderSnippet('dangling \u0001start only')}</p>);
    expect(container.querySelector('mark')).toBeNull();
    expect(container.textContent).toBe('dangling start only');
  });

  it('returns plain text untouched', () => {
    expect(renderSnippet('no matches here')).toEqual(['no matches here']);
  });
});

describe('CommandPalette — empty query (existing behavior unchanged)', () => {
  it('shows Actions and recent meetings by title, with no search tier and no backend search call', async () => {
    setupInvoke();
    await openPalette();

    // Actions group intact.
    expect(screen.getByText('Actions')).toBeInTheDocument();
    expect(screen.getByText('Start recording')).toBeInTheDocument();
    expect(screen.getByText('Go to Meetings')).toBeInTheDocument();
    expect(screen.getByText('Settings')).toBeInTheDocument();

    // Recent meetings by title, with dates.
    expect(screen.getByText('Meetings')).toBeInTheDocument();
    expect(screen.getByText('Design sync')).toBeInTheDocument();
    expect(screen.getByText('Kickoff planning')).toBeInTheDocument();

    // No full-text tier, no backend search, no <mark>s.
    expect(screen.queryByText('In transcripts, summaries & notes')).toBeNull();
    expect(document.querySelector('mark')).toBeNull();
    expect(searchCalls()).toHaveLength(0);

    // cmdk's own filtering stays on for the empty query (shouldFilter default).
    expect(document.querySelector('[cmdk-root]')).not.toBeNull();

    // Structural lock on the empty-query palette (specs/0033 acceptance:
    // Actions + recent meetings, byte-for-byte; specs/0035 appends the Ask-AI
    // row in its own heading-less group LAST, so it never steals cmdk's
    // first-item auto-selection from a search result).
    const groupHeadings = Array.from(
      document.querySelectorAll('[cmdk-group-heading]'),
    ).map((el) => el.textContent);
    expect(groupHeadings).toEqual(['Actions', 'Meetings']);
    const items = Array.from(document.querySelectorAll('[cmdk-item]')).map(
      (el) => el.textContent,
    );
    expect(items).toEqual([
      'Start recording⌘⇧R',
      'Go to Meetings',
      'Settings',
      'Design syncJun 24',
      'Kickoff planningJun 25',
      'Ask AI about your meetings…',
      'New to-do…',
    ]);
  });

  it('does not query the backend for a single-character query', async () => {
    setupInvoke();
    await openPalette();

    typeQuery('k');
    // Give the (should-not-exist) debounce a chance to fire.
    await new Promise((r) => setTimeout(r, 300));
    expect(searchCalls()).toHaveLength(0);
    expect(screen.queryByText('In transcripts, summaries & notes')).toBeNull();
  });
});

describe('CommandPalette — full-text search tier (specs/0033)', () => {
  it('debounces one backend call and renders hits with highlighted snippets', async () => {
    setupInvoke();
    await openPalette();

    typeQuery('kick');
    typeQuery('kickoff'); // rapid retype — earlier debounce must be cancelled

    await screen.findByText('In transcripts, summaries & notes');
    await waitFor(() => expect(searchCalls()).toHaveLength(1));
    expect(searchCalls()[0][1]).toEqual({ query: 'kickoff' });

    // Snippets render with <mark> spans (sentinels converted, not shown).
    const marks = Array.from(document.querySelectorAll('mark')).map((m) => m.textContent);
    expect(marks).toEqual(['kickoff', 'kickoff']);
    expect(document.body.textContent).not.toContain('\u0001');
    expect(document.body.textContent).not.toContain('\u0002');
    expect(screen.getByText(/is next week/)).toBeInTheDocument();
  });

  it('navigates to the meeting with a segment deep-link for transcript hits', async () => {
    setupInvoke();
    await openPalette();
    typeQuery('kickoff');
    await screen.findByText('In transcripts, summaries & notes');

    fireEvent.click(screen.getByText(/is next week/).closest('[cmdk-item]')!);
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=m2&segment=seg-42');
  });

  it('navigates without a segment param for non-transcript hits', async () => {
    setupInvoke();
    await openPalette();
    typeQuery('kickoff');
    await screen.findByText('In transcripts, summaries & notes');

    fireEvent.click(screen.getByText(/checklist/).closest('[cmdk-item]')!);
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=m1');
  });

  it('filters the title tier manually while the backend search is active', async () => {
    setupInvoke();
    await openPalette();
    typeQuery('kickoff');
    await screen.findByText('In transcripts, summaries & notes');

    // Title tier: only the matching meeting remains (exact item text — the hits
    // tier also mentions "Design sync", but with snippet text appended); actions
    // that don't match the query are gone.
    const itemTexts = Array.from(document.querySelectorAll('[cmdk-item]')).map(
      (el) => el.textContent,
    );
    expect(itemTexts).toContain('Kickoff planningJun 25');
    expect(itemTexts).not.toContain('Design syncJun 24');
    expect(screen.queryByText('Start recording')).toBeNull();
  });

  it('falls back to the title tier with a quiet warning when the backend errors', async () => {
    setupInvoke({ searchError: true });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await openPalette();

    typeQuery('kickoff');
    await waitFor(() => expect(searchCalls()).toHaveLength(1));
    await waitFor(() => expect(warnSpy).toHaveBeenCalled());

    // Palette still works: title tier matches, no search group, no crash.
    expect(screen.queryByText('In transcripts, summaries & notes')).toBeNull();
    expect(screen.getByText('Kickoff planning')).toBeInTheDocument();
  });
});

describe('CommandPalette — filter parity while backend search is active (cmdk defaultFilter)', () => {
  it("keeps fuzzy matches stable across the 2-char backend threshold ('s' → 'st' keeps Settings)", async () => {
    setupInvoke();
    await openPalette();

    // 1 char: cmdk's own command-score filtering (shouldFilter on).
    typeQuery('s');
    expect(screen.getByText('Settings')).toBeInTheDocument();

    // 2 chars: backend search takes over filtering — the manual pass must score
    // the same way (subsequence match), not plain substring ('st' ⊄ "Settings").
    typeQuery('st');
    await waitFor(() => expect(searchCalls()).toHaveLength(1));
    expect(screen.getByText('Settings')).toBeInTheDocument();
  });

  it('matches a pasted meeting id against its title row', async () => {
    setupInvoke();
    await openPalette();

    // The title tier's cmdk value is `${title} ${id}` — the manual pass scores
    // the same surface, so an id lookup keeps working past the 2-char threshold.
    typeQuery('m2');
    await waitFor(() => expect(searchCalls()).toHaveLength(1));

    const itemTexts = Array.from(document.querySelectorAll('[cmdk-item]')).map(
      (el) => el.textContent,
    );
    expect(itemTexts).toContain('Kickoff planningJun 25');
    expect(itemTexts).not.toContain('Design syncJun 24');
  });
});

describe('CommandPalette — meetings freshness (refetch on open, no failure latch)', () => {
  it('refetches on every open, so meetings recorded after the first open appear', async () => {
    const source = [...MEETINGS];
    invokeMock.mockImplementation((command: unknown) => {
      if (command === 'api_get_meetings') return Promise.resolve([...source]);
      if (command === 'api_search_meetings') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    await openPalette();

    togglePalette(); // close (dialog content unmounts)
    await waitFor(() => expect(screen.queryByText('Design sync')).toBeNull());

    // A meeting recorded while the palette was closed…
    source.push({ id: 'm3', title: 'Retro notes', createdAt: '2026-06-26T10:00:00Z' });
    togglePalette(); // reopen
    // …shows up in the title tier without an app restart.
    expect(await screen.findByText('Retro notes')).toBeInTheDocument();
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === 'api_get_meetings'),
    ).toHaveLength(2);
  });

  it('keeps the prior list rendered while a refresh is in flight', async () => {
    let call = 0;
    let resolveRefresh: ((value: unknown) => void) | undefined;
    invokeMock.mockImplementation((command: unknown) => {
      if (command === 'api_get_meetings') {
        call += 1;
        if (call === 1) return Promise.resolve(MEETINGS);
        return new Promise((resolve) => {
          resolveRefresh = resolve;
        });
      }
      return Promise.resolve(null);
    });
    await openPalette();

    togglePalette(); // close
    await waitFor(() => expect(screen.queryByText('Design sync')).toBeNull());
    togglePalette(); // reopen — the refresh hangs

    // The stale-but-useful list renders immediately; no empty flash.
    expect(await screen.findByText('Design sync')).toBeInTheDocument();

    await act(async () => {
      resolveRefresh?.(MEETINGS); // let the in-flight fetch settle cleanly
    });
  });

  it('does not latch a failed fetch — the next open retries and recovers', async () => {
    let fail = true;
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    invokeMock.mockImplementation((command: unknown) => {
      if (command === 'api_get_meetings') {
        return fail ? Promise.reject(new Error('db locked')) : Promise.resolve(MEETINGS);
      }
      return Promise.resolve(null);
    });
    render(<CommandPalette />);
    act(() => {
      openCommandPalette();
    });

    // First open: the fetch fails — actions still render, no Meetings group.
    await screen.findByText('Start recording');
    await waitFor(() => expect(errorSpy).toHaveBeenCalled());
    expect(screen.queryByText('Meetings')).toBeNull();

    togglePalette(); // close
    fail = false;
    togglePalette(); // reopen — retries instead of staying latched empty
    expect(await screen.findByText('Design sync')).toBeInTheDocument();
  });
});

describe('CommandPalette — Ask AI action (specs/0035)', () => {
  it('stays visible for a natural-language question that matches nothing else', async () => {
    setupInvoke();
    await openPalette();

    // A question fuzzy-matches neither the action label nor any meeting —
    // the Ask-AI action must survive both filtering passes (it consumes the
    // typed text as its input).
    typeQuery('what did we decide about pricing');
    await waitFor(() => expect(searchCalls().length).toBeGreaterThan(0));

    expect(screen.getByText('Ask AI about your meetings…')).toBeInTheDocument();
  });

  it('routes to /ask carrying the typed query, URL-encoded', async () => {
    setupInvoke();
    await openPalette();
    typeQuery('what did we decide about pricing');
    await waitFor(() => expect(searchCalls().length).toBeGreaterThan(0));

    fireEvent.click(screen.getByText('Ask AI about your meetings…').closest('[cmdk-item]')!);
    expect(pushMock).toHaveBeenCalledWith('/ask?q=what%20did%20we%20decide%20about%20pricing');
  });

  it('routes to /ask without a q param when nothing is typed', async () => {
    setupInvoke();
    await openPalette();

    fireEvent.click(screen.getByText('Ask AI about your meetings…').closest('[cmdk-item]')!);
    expect(pushMock).toHaveBeenCalledWith('/ask');
  });

  it('keeps the top search result FIRST for a content query — Enter opens it, not Ask-AI (0033 type-then-Enter)', async () => {
    setupInvoke();
    await openPalette();
    typeQuery('kickoff');
    await screen.findByText('In transcripts, summaries & notes');

    // Ask-AI lives in its own LAST group, so with shouldFilter=false cmdk's
    // first-DOM-item auto-selection stays on the top result.
    const items = Array.from(document.querySelectorAll('[cmdk-item]')).map(
      (el) => el.textContent,
    );
    expect(items[0]).toBe('Kickoff planningJun 25');
    // Ask-AI and the New-to-do action ride along at the END (their own group), so cmdk's
    // first-DOM-item auto-selection stays on the top search result.
    expect(items).toContain('Ask AI about your meetings…');
    expect(items[items.length - 1]).toBe('New to-do: “kickoff”');

    await waitFor(() => {
      const selected = document.querySelector('[cmdk-item][aria-selected="true"]');
      expect(selected?.textContent).toBe('Kickoff planningJun 25');
    });
  });

  it('shows the Ask row WITHOUT a contradictory "No results found." when nothing matches at all', async () => {
    invokeMock.mockImplementation((command: unknown) => {
      if (command === 'api_get_meetings') return Promise.resolve(MEETINGS);
      if (command === 'api_search_meetings') return Promise.resolve([]); // no hits
      return Promise.resolve(null);
    });
    await openPalette();

    typeQuery('zzqx totally unrelated'); // matches no action, meeting, or hit
    await waitFor(() => expect(searchCalls()).toHaveLength(1));

    // The always-visible Ask row is the only actionable item left…
    expect(screen.getByText('Ask AI about your meetings…')).toBeInTheDocument();
    // …and the empty state is coherent: the scoped message, never cmdk's
    // store-count-based "No results found." next to a visible row.
    expect(screen.queryByText('No results found.')).toBeNull();
    expect(screen.getByText('No matching meetings')).toBeInTheDocument();
  });
});
