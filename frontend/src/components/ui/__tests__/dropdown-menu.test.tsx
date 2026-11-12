import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { Trash2 } from '@/components/ui/icons';

function openMenu(items: React.ReactNode) {
  return render(
    <DropdownMenu open>
      <DropdownMenuTrigger>Open</DropdownMenuTrigger>
      <DropdownMenuContent>{items}</DropdownMenuContent>
    </DropdownMenu>
  );
}

describe('DropdownMenuItem', () => {
  it('renders a leading icon at the standard 16px with the menu gap', () => {
    openMenu(<DropdownMenuItem icon={Trash2}>Delete</DropdownMenuItem>);
    const item = screen.getByRole('menuitem', { name: 'Delete' });
    const svg = item.querySelector('svg')!;
    expect(svg).toBeTruthy();
    expect(svg.getAttribute('width')).toBe('16');
    expect(svg.getAttribute('class')).toContain('mr-2');
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    // The icon comes before the label.
    expect(item.firstChild).toBe(svg);
  });

  it('renders no icon when none is given', () => {
    openMenu(<DropdownMenuItem>Rename</DropdownMenuItem>);
    expect(screen.getByRole('menuitem', { name: 'Rename' }).querySelector('svg')).toBeNull();
  });

  it('has a destructive variant', () => {
    openMenu(
      <>
        <DropdownMenuItem variant="destructive">Forget</DropdownMenuItem>
        <DropdownMenuItem>Keep</DropdownMenuItem>
      </>
    );
    const bad = screen.getByRole('menuitem', { name: 'Forget' });
    expect(bad.className).toContain('text-destructive');
    expect(bad.className).toContain('focus:text-destructive');
    expect(bad.getAttribute('data-variant')).toBe('destructive');
    expect(screen.getByRole('menuitem', { name: 'Keep' }).className).not.toContain('text-destructive');
  });
});
