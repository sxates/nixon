import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { PrepPanel } from '@/components/MeetingDetails/PrepPanel';
import type { PrepView } from '@/lib/prep';
import type { ActionItem, Person } from '@/types';

// specs/0036 — the Prep tab body. Locks the brief states (loading spinner while
// pending/absent, the rendered brief when ready, the friendly empty state when there
// are no prior occurrences) and the carried-over open items split into mine vs. owed
// by others (with people resolved to display names). Mocks invoke + safeListen the
// same way the other MeetingDetails component tests do.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn(() => () => {}) }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
// The prep-notes editor pulls in a contentEditable surface + markdown round-trip that
// isn't under test here — stub it to a marker so PrepPanel is tested in isolation.
vi.mock('@/components/MeetingDetails/PrepNotesEditor', () => ({
  PrepNotesEditor: () => <div data-testid="prep-notes-editor" />,
}));

import { invoke } from '@tauri-apps/api/core';

const invokeMock = vi.mocked(invoke);

function makeView(overrides: Partial<PrepView> = {}): PrepView {
  return {
    meetingId: 'm1',
    origin: 'scheduled',
    title: 'Weekly design review',
    briefStatus: 'ready',
    briefMarkdown: null,
    briefSources: [],
    openItems: [],
    prepNotesMarkdown: null,
    prepNotesJson: null,
    linkedMeetings: [],
    ...overrides,
  };
}

function makeItem(overrides: Partial<ActionItem> = {}): ActionItem {
  return {
    id: 'ai-1',
    meetingId: 'm-prev',
    description: 'Do the thing',
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: null,
    dueDate: null,
    status: 'open',
    source: 'extracted',
    userEdited: false,
    contentKey: 'ck1',
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    ...overrides,
  };
}

function person(id: string, displayName: string): Person {
  return {
    id,
    email: null,
    displayName,
    role: null,
    notes: null,
    voiceprintOptOut: false,
    starred: false,
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
  };
}

/** Route the invoke mock by command; unrouted commands resolve to a benign default. */
function stubInvoke(handlers: Record<string, (args?: unknown) => unknown>) {
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in handlers) return handlers[cmd]!(args);
    if (cmd === 'api_list_people') return [];
    throw new Error(`Unexpected command in test: ${cmd}`);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('PrepPanel (specs/0036)', () => {
  it('shows a spinner while the brief is pending', async () => {
    stubInvoke({ api_get_prep: () => makeView({ briefStatus: 'pending' }) });
    render(<PrepPanel meetingId="m1" />);

    expect(await screen.findByText('Preparing your brief…')).toBeInTheDocument();
    expect(screen.getByRole('status')).toBeInTheDocument();
  });

  it('shows a spinner while the brief is absent (generation just kicked off)', async () => {
    stubInvoke({ api_get_prep: () => makeView({ briefStatus: 'absent' }) });
    render(<PrepPanel meetingId="m1" />);

    expect(await screen.findByText('Preparing your brief…')).toBeInTheDocument();
  });

  it('renders the brief when ready', async () => {
    stubInvoke({
      api_get_prep: () =>
        makeView({
          briefStatus: 'ready',
          briefMarkdown: '## Where we left off\nWe shipped the v2 redesign.',
          briefSources: [
            { meetingId: 'm-prev', title: 'Prev review', createdAt: '2026-06-20T10:00:00Z', cited: true },
          ],
        }),
    });
    render(<PrepPanel meetingId="m1" />);

    expect(await screen.findByText('We shipped the v2 redesign.')).toBeInTheDocument();
    // The cited source is listed under "From".
    expect(screen.getByText('Prev review')).toBeInTheDocument();
    // Regenerate is offered once a brief exists.
    expect(screen.getByRole('button', { name: /regenerate/i })).toBeInTheDocument();
  });

  it('explains the empty state and offers the link action when there are no prior occurrences', async () => {
    stubInvoke({ api_get_prep: () => makeView({ briefStatus: 'none' }) });
    render(<PrepPanel meetingId="m1" />);

    // specs/0041 WS4: say WHY prep is empty + offer manual association, instead of silence.
    expect(
      await screen.findByText(/No previous meetings found for this series/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /link previous meeting/i }),
    ).toBeInTheDocument();
    // Nothing to regenerate → no button.
    expect(screen.queryByRole('button', { name: /regenerate/i })).not.toBeInTheDocument();
  });

  it('splits carried-over open items into mine vs. owed by others', async () => {
    stubInvoke({
      api_get_prep: () =>
        makeView({
          briefStatus: 'none',
          openItems: [
            makeItem({ id: 'mine-1', assigneeIsSelf: true, description: 'Draft the roadmap' }),
            makeItem({
              id: 'other-1',
              assigneeIsSelf: false,
              assigneePersonId: 'p1',
              description: 'Send the budget',
            }),
          ],
        }),
      api_list_people: () => [person('p1', 'Alice')],
    });
    render(<PrepPanel meetingId="m1" />);

    expect(await screen.findByText('Draft the roadmap')).toBeInTheDocument();
    expect(screen.getByText('Your open items')).toBeInTheDocument();

    // The "others" item is grouped under the resolved assignee name (people load async).
    expect(await screen.findByText('Send the budget')).toBeInTheDocument();
    expect(screen.getByText('Owed by others')).toBeInTheDocument();
    expect(await screen.findByText('Alice')).toBeInTheDocument();
  });

  // ── specs/0041 WS4: manual series association ────────────────────────────

  it('opens the picker, filters by title, and links the picked meeting', async () => {
    stubInvoke({
      api_get_prep: () => makeView({ briefStatus: 'none' }),
      api_get_meetings: () => [
        { id: 'm-old-1', title: 'Design catch-up', createdAt: '2026-07-01T10:00:00Z' },
        { id: 'm-old-2', title: 'Budget review', createdAt: '2026-07-02T10:00:00Z' },
        // The panel's own meeting must never be offered.
        { id: 'm1', title: 'Weekly design review', createdAt: '2026-07-03T10:00:00Z' },
      ],
      api_link_meeting_to_series: () => null,
    });
    render(<PrepPanel meetingId="m1" />);

    fireEvent.click(
      await screen.findByRole('button', { name: /link previous meeting/i }),
    );

    // Recent meetings listed (self excluded), then narrowed by the title filter.
    expect(await screen.findByText('Design catch-up')).toBeInTheDocument();
    expect(screen.getByText('Budget review')).toBeInTheDocument();
    expect(screen.queryByText('Weekly design review')).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/filter meetings by title/i), {
      target: { value: 'design' },
    });
    expect(screen.getByText('Design catch-up')).toBeInTheDocument();
    expect(screen.queryByText('Budget review')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('Design catch-up'));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_link_meeting_to_series', {
        meetingId: 'm-old-1',
        targetMeetingId: 'm1',
      }),
    );
    // Successful link closes the picker and re-reads the prep view.
    await waitFor(() =>
      expect(screen.queryByText('Design catch-up')).not.toBeInTheDocument(),
    );
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'api_get_prep').length,
    ).toBeGreaterThanOrEqual(2);
  });

  it('lists linked meetings with an unlink affordance', async () => {
    stubInvoke({
      api_get_prep: () =>
        makeView({
          briefStatus: 'ready',
          briefMarkdown: 'Where we left off.',
          linkedMeetings: [
            { id: 'm-linked', title: 'Old design chat', startedAt: '2026-06-30T10:00:00Z' },
          ],
        }),
      api_unlink_meeting_from_series: () => null,
    });
    render(<PrepPanel meetingId="m1" />);

    expect(await screen.findByText('Linked meetings')).toBeInTheDocument();
    expect(screen.getByText('Old design chat')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /unlink old design chat/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_unlink_meeting_from_series', {
        meetingId: 'm-linked',
      }),
    );
  });
});
