import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SIDEBAR_ROW } from '@/components/Sidebar/row';

// ---- Mocks ---------------------------------------------------------------
// The sidebar's own provider pulls in Tauri IPC + sonner; the component only needs
// the handful of fields it destructures, so stub `useSidebar` wholesale.
const sidebarState = {
  isCollapsed: false,
  toggleCollapse: vi.fn(),
  handleRecordingToggle: vi.fn(),
  handleNewNote: vi.fn(async () => {}),
};
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => sidebarState,
}));

let pathname = '/meetings';
const push = vi.fn();
vi.mock('next/navigation', () => ({
  usePathname: () => pathname,
  useRouter: () => ({ push }),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));

vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));

// specs/0064 W5 — the sidebar now carries the queue row, which reads the backlog, LLM
// activity and transcript contexts. This suite is about the nav chrome, so they are stubbed
// to their idle shapes; QueueRow has its own suite.
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({
    view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    stop: vi.fn(),
    startNow: vi.fn(),
    dismissDone: vi.fn(),
    enqueueMeeting: vi.fn(),
  }),
}));
vi.mock('@/contexts/LlmActivityProvider', () => ({
  useOptionalLlmActivity: () => ({ running: [], history: [], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() }),
}));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => ({ meetingTitle: null }) }));
vi.mock('@/contexts/QueueOpenContext', async () => {
  const react = await import('react');
  return { useQueueOpen: () => { const [open, setOpen] = react.useState(false); return { open, setOpen }; } };
});

// specs/0069 review, fix round 2 — this suite previously mocked no `UpdateStatusContext`, so
// `useOptionalUpdateStatus()` returned null and `UpdateRow` bailed out of every render before
// rendering anything. The parity tests below compare "every slot's row classes across both
// states" and "no orphan row" — with the update row absent, they never looked at the one row
// this branch changed the most. A `ready` status makes it render (with a `RestartToUpdateButton`)
// in both the expanded and collapsed variants, like every other footer row.
vi.mock('@/contexts/UpdateStatusContext', () => ({
  useOptionalUpdateStatus: () => ({
    status: { state: 'ready', version: '9.9.9', notes: '', last_checked: null },
    busy: false,
    error: null,
    checkNow: vi.fn(),
    install: vi.fn(),
  }),
}));

import Sidebar from '@/components/Sidebar';

// specs/0064 W5 — 'Home' is now 'Today', matching the screen it opens.
const NAV_NAMES = ['Today', 'All meetings', 'Action items', 'People', 'Ask AI'];

describe('Sidebar (specs/0057 Plan 3 Task 3)', () => {
  beforeEach(() => {
    pathname = '/meetings';
    sidebarState.isCollapsed = false;
  });

  it('renders all five nav destinations when expanded', () => {
    render(<Sidebar />);
    for (const name of NAV_NAMES) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
  });

  it('marks the active destination with aria-current="page" when expanded', () => {
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'All meetings' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('button', { name: 'Today' })).not.toHaveAttribute('aria-current');
  });

  it('shows the engraved NIXON wordmark when expanded', () => {
    render(<Sidebar />);
    expect(screen.getByText('NIXON')).toBeInTheDocument();
  });

  it('no longer renders the "N" avatar puck', () => {
    render(<Sidebar />);
    expect(screen.queryByText(/^N$/)).toBeNull();
  });

  it('renders the same five destinations as labelled icon buttons when collapsed', () => {
    sidebarState.isCollapsed = true;
    render(<Sidebar />);
    for (const name of NAV_NAMES) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
  });

  it('marks the active destination with aria-current="page" when collapsed', () => {
    sidebarState.isCollapsed = true;
    pathname = '/tasks';
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'Action items' })).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  it('keeps a settings entry in both branches', () => {
    const { unmount } = render(<Sidebar />);
    expect(screen.getByRole('button', { name: /settings/i })).toBeInTheDocument();
    unmount();
    sidebarState.isCollapsed = true;
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: /settings/i })).toBeInTheDocument();
  });
});

function slotOrder(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('[data-sidebar-slot]')).map(
    (el) => el.getAttribute('data-sidebar-slot') as string,
  );
}

// The slot span's own className is the hardcoded `SIDEBAR_ICON_SLOT` constant, so comparing
// it against itself can never fail (review finding, fix round 1). What actually carries
// `SIDEBAR_ROW` — and so is where a stray `h-8`/`mb-1`/`pl-5` would come back — is the row
// the slot lives in. That row is marked `data-sidebar-row` rather than inferred as the slot's
// parent: when the wordmark moved up beside the hamburger (owner request, 2026-09-20) the
// toggle stopped being the row and became a child of it, and a `parentElement` lookup silently
// started measuring the button instead. Keying by slot name makes a mismatch name the
// offending row instead of just diffing two opaque arrays.
function rowClassesBySlot(container: HTMLElement): Record<string, string> {
  const rows: Record<string, string> = {};
  container.querySelectorAll('[data-sidebar-slot]').forEach((el) => {
    const slot = el.getAttribute('data-sidebar-slot') as string;
    const row = el.closest('[data-sidebar-row]');
    expect(row, `slot "${slot}" is not inside a [data-sidebar-row]`).not.toBeNull();
    rows[slot] = row?.className ?? '';
  });
  return rows;
}

const ROW_TOKENS = SIDEBAR_ROW.split(' ');

// specs/0069 review, fix round 2 — the containment check above passes even when one state
// carries an extra box-model class the other doesn't (that is exactly how the old
// `collapsed && 'mb-1'` shipped: `mb-1` is not one of `ROW_TOKENS`, so its presence on only
// the collapsed branch was invisible to a "contains" assertion). This pulls out just the
// margin/padding/height/width/gap utilities — the ones that can move or resize a row — and
// requires the two states to carry the *same set*, so an asymmetric one is caught regardless
// of which side it landed on. It deliberately ignores non-geometry classes (e.g. QueueRow's
// expanded-only `relative text-left`), which don't move anything.
const GEOMETRY_CLASS_RE = /^-?(?:m|p)[trblxy]?-|^(?:h|w)-|^gap(?:-[xy])?-|^inset(?:-[xy])?-/;

function geometryTokens(className: string): string[] {
  return className.split(/\s+/).filter((t) => GEOMETRY_CLASS_RE.test(t)).sort();
}

describe('Sidebar parity (specs/0069 W1)', () => {
  it('renders the same icon column, in the same order, in both states', () => {
    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    const expandedSlots = slotOrder(expanded.container);
    expect(expandedSlots.length).toBeGreaterThan(5);
    expanded.unmount();

    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    expect(slotOrder(collapsed.container)).toEqual(expandedSlots);
  });

  it("gives every slot's ROW the shared row classes, in both states", () => {
    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    const expandedRows = rowClassesBySlot(expanded.container);
    expanded.unmount();

    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    const collapsedRows = rowClassesBySlot(collapsed.container);

    const slots = Object.keys(expandedRows);
    expect(slots.length).toBeGreaterThan(5);
    for (const slot of slots) {
      for (const token of ROW_TOKENS) {
        expect(expandedRows[slot].split(/\s+/), `expanded "${slot}" row missing "${token}"`).toContain(
          token,
        );
        expect(
          (collapsedRows[slot] ?? '').split(/\s+/),
          `collapsed "${slot}" row missing "${token}"`,
        ).toContain(token);
      }
    }
  });

  it("gives every slot's row the same box-model classes in both states (no asymmetric mb-1/pl/h-8)", () => {
    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    const expandedRows = rowClassesBySlot(expanded.container);
    expanded.unmount();

    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    const collapsedRows = rowClassesBySlot(collapsed.container);

    const slots = Object.keys(expandedRows);
    expect(slots.length).toBeGreaterThan(5);
    for (const slot of slots) {
      expect(geometryTokens(collapsedRows[slot] ?? ''), `"${slot}" row geometry differs between states`).toEqual(
        geometryTokens(expandedRows[slot] ?? ''),
      );
    }
  });

  it('shows NIXON beside the hamburger when expanded, and no reel-mark row in either state', () => {
    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    expect(collapsed.container.querySelector('[data-sidebar-slot="mark"]')).toBeNull();
    expect(screen.queryByText('NIXON')).toBeNull();
    collapsed.unmount();

    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    expect(expanded.container.querySelector('[data-sidebar-slot="mark"]')).toBeNull();
    // The wordmark rides in the hamburger's row, so it is a sibling of the toggle — not its
    // label. A button reading NIXON but announced as "Collapse sidebar" fails WCAG 2.5.3.
    const toggle = screen.getByRole('button', { name: /collapse sidebar/i });
    expect(toggle).not.toHaveTextContent('NIXON');
    expect(toggle.closest('[data-sidebar-row]')).toHaveTextContent('NIXON');
  });

  it('has no divider above the footer rows', () => {
    sidebarState.isCollapsed = false;
    const { container } = render(<Sidebar />);
    expect(container.querySelector('.border-t')).toBeNull();
  });

  it('no longer offers Import audio in either state (specs/0069 W2)', () => {
    sidebarState.isCollapsed = false;
    const { unmount } = render(<Sidebar />);
    expect(screen.queryByRole('button', { name: /import audio/i })).toBeNull();
    unmount();
    sidebarState.isCollapsed = true;
    render(<Sidebar />);
    expect(screen.queryByRole('button', { name: /import audio/i })).toBeNull();
  });
});
