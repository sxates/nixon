import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';

function renderTabs(badge?: React.ReactNode) {
  return render(
    <Tabs defaultValue="summary">
      <TabsList aria-label="Person views">
        <TabsTrigger value="summary">Summary</TabsTrigger>
        <TabsTrigger value="meetings" badge={badge}>
          Meetings
        </TabsTrigger>
      </TabsList>
      <TabsContent value="summary">S</TabsContent>
      <TabsContent value="meetings">M</TabsContent>
    </Tabs>
  );
}

// docs/design-system.md — underline tabs are the default; no screen overrides the pill.
describe('Tabs', () => {
  it('draws the list as a hairline rule, not the shadcn pill', () => {
    renderTabs();
    const list = screen.getByRole('tablist');
    expect(list.className).toContain('border-b');
    expect(list.className).toContain('border-border');
    expect(list.className).not.toContain('bg-muted');
    expect(list.className).not.toContain('rounded-lg');
  });

  it('underlines the active trigger in brand', () => {
    renderTabs();
    const active = screen.getByRole('tab', { name: 'Summary' });
    expect(active.getAttribute('data-state')).toBe('active');
    expect(active.className).toContain('border-b-2');
    expect(active.className).toContain('data-[state=active]:border-brand');
    expect(active.className).toContain('font-semibold');
    expect(active.className).not.toContain('data-[state=active]:shadow');
  });

  it('renders a numeric badge as a count chip', () => {
    renderTabs(4);
    const tab = screen.getByRole('tab', { name: /Meetings/ });
    const chip = tab.querySelector('[data-tab-badge]')!;
    expect(chip.textContent).toBe('4');
    expect(chip.className).toContain('text-caption');
    expect(chip.className).toContain('group-data-[state=active]:bg-brand-wash');
  });

  it('renders a node badge as given', () => {
    renderTabs(<span data-testid="dot" />);
    expect(screen.getByTestId('dot').parentElement!.hasAttribute('data-tab-badge')).toBe(true);
  });

  it('renders no badge by default', () => {
    const { container } = renderTabs();
    expect(container.querySelector('[data-tab-badge]')).toBeNull();
  });
});
