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
vi.mock('@/contexts/ImportDialogContext', () => ({
  useImportDialog: () => ({ openImportDialog: vi.fn() }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ betaFeatures: { importAndRetranscribe: false } }),
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
// element the slot lives in (the `<button>`/`<div>` one level up). Keying by slot name makes
// a mismatch name the offending row instead of just diffing two opaque arrays.
function rowClassesBySlot(container: HTMLElement): Record<string, string> {
  const rows: Record<string, string> = {};
  container.querySelectorAll('[data-sidebar-slot]').forEach((el) => {
    const slot = el.getAttribute('data-sidebar-slot') as string;
    rows[slot] = el.parentElement?.className ?? '';
  });
  return rows;
}

const ROW_TOKENS = SIDEBAR_ROW.split(' ');

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

  it('gives every slot the same column class in both states', () => {
    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    const cls = Array.from(expanded.container.querySelectorAll('[data-sidebar-slot]')).map(
      (el) => el.className,
    );
    expanded.unmount();
    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    expect(
      Array.from(collapsed.container.querySelectorAll('[data-sidebar-slot]')).map(
        (el) => el.className,
      ),
    ).toEqual(cls);
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

  it('shows the reel mark in both states and NIXON only when expanded', () => {
    sidebarState.isCollapsed = true;
    const collapsed = render(<Sidebar />);
    expect(collapsed.container.querySelector('[data-sidebar-slot="mark"]')).not.toBeNull();
    expect(screen.queryByText('NIXON')).toBeNull();
    collapsed.unmount();

    sidebarState.isCollapsed = false;
    const expanded = render(<Sidebar />);
    expect(expanded.container.querySelector('[data-sidebar-slot="mark"]')).not.toBeNull();
    expect(screen.getByText('NIXON')).toBeInTheDocument();
  });

  it('has no divider above the footer rows', () => {
    sidebarState.isCollapsed = false;
    const { container } = render(<Sidebar />);
    expect(container.querySelector('.border-t')).toBeNull();
  });
});
