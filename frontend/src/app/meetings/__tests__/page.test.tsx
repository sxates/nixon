import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';

// specs/0038 WS8 — the All Meetings row now shows a bounded attendee preview
// (avatars + "Name, +N"), with the device owner excluded DISPLAY-ONLY so a 1:1
// reads as the single other person.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ refetchMeetings: vi.fn(), activeRecordingMeetingId: null }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
vi.mock('@/components/MeetingDetails/DeleteMeetingDialog', () => ({
  DeleteMeetingDialog: () => null,
}));
const openImportDialog = vi.fn();
vi.mock('@/contexts/ImportDialogContext', () => ({
  useImportDialog: () => ({ openImportDialog }),
}));
// framer-motion `motion.div` → passthrough so jsdom renders children immediately.
// Each tag (motion.div, motion.span, …) must resolve to the SAME component
// reference across renders — a Proxy `get` trap that mints a fresh function on
// every property access makes React treat each render's `<motion.div>` as a
// different component type, unmounting and remounting the subtree (and
// detaching any node a test captured earlier). Cache per-key so repeat access
// to the same tag is stable.
vi.mock('framer-motion', () => {
  const cache = new Map<string | symbol, unknown>();
  const passthrough =
    ({ children, ...props }: { children?: React.ReactNode }) => <div {...props}>{children}</div>;
  return {
    motion: new Proxy(
      {},
      {
        get: (_target, key) => {
          if (!cache.has(key)) cache.set(key, passthrough);
          return cache.get(key);
        },
      },
    ),
  };
});

import { invoke } from '@tauri-apps/api/core';
import AllMeetingsPage from '@/app/meetings/page';

const invokeMock = vi.mocked(invoke);

const now = new Date().toISOString();

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  window.localStorage.clear();
});

describe('All Meetings attendee preview (specs/0038 WS8)', () => {
  it('renders a bounded preview with a "+N" overflow (owner-excluded count)', async () => {
    invokeMock.mockResolvedValue([
      {
        id: 'm-many',
        title: 'Quarterly Planning',
        createdAt: now,
        attendees: [
          { name: 'Alice Smith', email: 'alice@x.com', isCurrentUser: false },
          { name: 'Bob Lee', email: 'bob@x.com', isCurrentUser: false },
          { name: 'Carol Ng', email: 'carol@x.com', isCurrentUser: false },
        ],
        attendeeCount: 5,
      },
    ]);

    render(<AllMeetingsPage />);

    // maxNames=1 on the row → "Alice Smith, +4" (5 total minus the 1 named).
    expect(await screen.findByText('Alice Smith, +4')).toBeInTheDocument();
    // Three avatars are shown (the preview cap), each labelled by its person.
    expect(screen.getByTitle('Alice Smith · alice@x.com')).toBeInTheDocument();
    expect(screen.getByTitle('Carol Ng · carol@x.com')).toBeInTheDocument();
  });

  it('a 1:1 shows one (non-owner) attendee after the owner filter', async () => {
    invokeMock.mockResolvedValue([
      {
        id: 'm-1on1',
        title: 'Alice / Me',
        createdAt: now,
        // Owner-inclusive preview (the rare manually-added "You" case) — the frontend
        // must drop the owner from both the avatars and the count.
        attendees: [
          { name: 'Me', email: 'me@x.com', isCurrentUser: true },
          { name: 'Dana Cruz', email: 'dana@x.com', isCurrentUser: false },
        ],
        attendeeCount: 2,
      },
    ]);

    render(<AllMeetingsPage />);

    // Just the other person, no "+N", no "You".
    expect(await screen.findByText('Dana Cruz')).toBeInTheDocument();
    expect(screen.queryByText(/\+\d/)).not.toBeInTheDocument();
    expect(screen.queryByText('You')).not.toBeInTheDocument();
    // The owner avatar is not rendered; the other person's is.
    expect(screen.queryByTitle('Me · me@x.com')).not.toBeInTheDocument();
    expect(screen.getByTitle('Dana Cruz · dana@x.com')).toBeInTheDocument();
  });
});

describe('All Meetings search entry point (specs/0054 W3 follow-up)', () => {
  it('opens the ⌘K palette from the header button', async () => {
    // Search moved off the sidebar and onto this page: this is where someone is
    // standing when they cannot find a meeting. It must reach the same palette.
    invokeMock.mockResolvedValue([]);
    const onOpen = vi.fn();
    window.addEventListener('nixon:open-command-palette', onOpen);

    const { getByRole } = render(<AllMeetingsPage />);
    // Query through the render result rather than `screen`: this file renders the
    // page in several tests, and a document-wide lookup can resolve against an
    // already-unmounted tree, whose React handlers are gone.
    fireEvent.click(getByRole('button', { name: /search meetings/i }));

    expect(onOpen).toHaveBeenCalled();
    window.removeEventListener('nixon:open-command-palette', onOpen);
  });
});

describe('All Meetings view toggle (specs/0057 Task 2)', () => {
  it('exposes Month/List as an aria-pressed group, and pressing Month flips the state', async () => {
    // The toggle moved from a pair of Buttons to the shared SegmentedControl.
    // The pressed contract is what assistive tech reads, so it is what we pin.
    invokeMock.mockResolvedValue([]);
    const { getByRole } = render(<AllMeetingsPage />);

    // Re-query every time: the framer-motion mock returns a fresh component type
    // per render, so each state change replaces the DOM subtree and any node
    // captured earlier is detached (same trap the search test above calls out).
    const option = (name: RegExp) =>
      within(getByRole('group', { name: /meeting view/i })).getByRole('button', { name });

    // List is the default view (specs/0054 W3).
    expect(option(/list/i).getAttribute('aria-pressed')).toBe('true');
    expect(option(/month/i).getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(option(/month/i));

    expect(option(/month/i).getAttribute('aria-pressed')).toBe('true');
    expect(option(/list/i).getAttribute('aria-pressed')).toBe('false');
  });
});

describe('All Meetings tape log (specs/0057 Task 6)', () => {
  it('prints the reel tag on the row when the meeting has a reel ordinal', async () => {
    // The list reads like a shelf of tapes: every line is stamped with its reel
    // number, so a row can be matched to the archive by eye.
    invokeMock.mockResolvedValue([
      { id: 'm-reel', title: 'Standup', createdAt: now, reelNumber: 1, durationSeconds: 2538 },
    ]);

    const { findByText } = render(<AllMeetingsPage />);

    expect(await findByText('R0001')).toBeInTheDocument();
  });

  it('leaves the reel column blank for a meeting with no ordinal', async () => {
    invokeMock.mockResolvedValue([{ id: 'm-none', title: 'Ad hoc', createdAt: now }]);

    const { findByText, queryByText } = render(<AllMeetingsPage />);

    await findByText('Ad hoc');
    expect(queryByText(/^R\d+$/)).not.toBeInTheDocument();
  });
});

describe('All Meetings row grid tracks (fix round 1, Important 1)', () => {
  it('keeps 5 direct grid children whether or not the row has attendees', async () => {
    // display:none on the attendee cell (the old `hidden … sm:flex`) removes a grid
    // ITEM below `sm`, so a row WITH attendees auto-placed into 4 tracks while a
    // row without kept 5 — the tape log misaligned. The wrapper must always be a
    // grid item; only its contents may collapse.
    invokeMock.mockResolvedValue([
      {
        id: 'm-attendees',
        title: 'Standup',
        createdAt: now,
        durationSeconds: 2538,
        attendees: [{ name: 'Alice Smith', email: 'alice@x.com', isCurrentUser: false }],
        attendeeCount: 1,
      },
      { id: 'm-none', title: 'Ad hoc', createdAt: now, durationSeconds: 120 },
    ]);

    render(<AllMeetingsPage />);

    const withAttendees = (await screen.findByText('Standup')).closest('button')!;
    const withoutAttendees = (await screen.findByText('Ad hoc')).closest('button')!;

    expect(withAttendees.children).toHaveLength(5);
    expect(withoutAttendees.children).toHaveLength(5);
  });
});

describe('All Meetings row decorative counter (fix round 1, Important 2)', () => {
  it("the row's accessible name reads the human duration, not the digit-well timer", async () => {
    invokeMock.mockResolvedValue([
      { id: 'm-reel', title: 'Standup', createdAt: now, reelNumber: 1, durationSeconds: 2538 },
    ]);

    render(<AllMeetingsPage />);

    const row = await screen.findByRole('button', { name: /Standup.*42 min/ });
    expect(row.textContent).not.toMatch(/Elapsed time/);
    // The visible digit-well counter must not itself be an accessible timer landmark.
    expect(within(row).queryByRole('timer')).not.toBeInTheDocument();
  });
});

describe('All Meetings import button (specs/0069 W2)', () => {
  it('opens the import dialog from the header (specs/0069 W2)', () => {
    invokeMock.mockResolvedValue([]);
    render(<AllMeetingsPage />);
    fireEvent.click(screen.getByRole('button', { name: /import audio/i }));
    expect(openImportDialog).toHaveBeenCalledTimes(1);
  });
});
