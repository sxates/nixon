import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

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

import Sidebar from '@/components/Sidebar';

const NAV_NAMES = ['Home', 'All meetings', 'Action items', 'People', 'Ask AI'];

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
    expect(screen.getByRole('button', { name: 'Home' })).not.toHaveAttribute('aria-current');
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
