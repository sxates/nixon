import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import MainContent from '@/components/MainContent';

vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ isCollapsed: true }),
}));

// specs/0057 Plan 2 Task 2 — the persistent transport rail is fixed to the bottom edge, so
// the main column reserves its height; otherwise page footers hide behind the rail.
describe('MainContent', () => {
  it('reserves the rail height at the bottom', () => {
    const { container } = render(<MainContent><div>x</div></MainContent>);
    const main = container.querySelector('main')!;
    expect(main.className).toContain('pb-[var(--rail-h)]');
  });
});
