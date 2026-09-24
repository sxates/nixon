import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen } from '@testing-library/react';

let pathname = '/';
vi.mock('next/navigation', () => ({
  usePathname: () => pathname,
  useRouter: () => ({ push: vi.fn() }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));

import { SidebarProvider, useSidebar } from '../SidebarProvider';

let narrow = false;
beforeEach(() => {
  pathname = '/';
  window.localStorage.setItem('nixon.sidebar.collapsed', '0'); // saved preference: expanded
  window.matchMedia = vi.fn((query: string) => ({
    matches: narrow,
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
});

function Probe() {
  const { isCollapsed, isContentInsetCollapsed, toggleCollapse } = useSidebar();
  return (
    <button onClick={toggleCollapse}>
      {`panel:${isCollapsed ? 'rail' : 'open'} page:${isContentInsetCollapsed ? 'rail' : 'column'}`}
    </button>
  );
}

// Owner feedback 2026-09-23: a narrow window should collapse the sidebar.
describe('SidebarProvider in a narrow window', () => {
  it('a wide window honours the saved expanded preference', () => {
    narrow = false;
    render(<SidebarProvider><Probe /></SidebarProvider>);
    expect(screen.getByRole('button')).toHaveTextContent('panel:open page:column');
  });

  it('a narrow window collapses to the rail whatever was saved', () => {
    narrow = true;
    render(<SidebarProvider><Probe /></SidebarProvider>);
    expect(screen.getByRole('button')).toHaveTextContent('panel:rail page:rail');
  });

  it('expanding in a narrow window overlays the page and is not saved', () => {
    narrow = true;
    window.localStorage.setItem('nixon.sidebar.collapsed', '1');
    render(<SidebarProvider><Probe /></SidebarProvider>);
    act(() => screen.getByRole('button').click());
    expect(screen.getByRole('button')).toHaveTextContent('panel:open page:rail');
    expect(window.localStorage.getItem('nixon.sidebar.collapsed')).toBe('1');
  });
});
