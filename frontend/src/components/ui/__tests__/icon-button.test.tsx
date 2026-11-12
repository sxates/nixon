import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { IconButton } from '@/components/ui/icon-button';
import { MenuButton } from '@/components/ui/menu-button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';

describe('IconButton', () => {
  it('takes its accessible name from label', () => {
    render(<IconButton label="Regenerate" icon={<svg />} />);
    expect(screen.getByRole('button', { name: 'Regenerate' })).toBeTruthy();
  });

  it('defaults to the 28px bordered key (the pre-wave-0a look)', () => {
    render(<IconButton label="Regenerate" icon={<svg />} />);
    const cls = screen.getByRole('button').className;
    expect(cls).toContain('h-7');
    expect(cls).toContain('w-7');
    expect(cls).toContain('border-border');
    expect(cls).toContain('rounded-md');
    expect(cls).toContain('focus-visible:ring-2');
  });

  it('has a 32px md size', () => {
    render(<IconButton label="Back" size="md" icon={<svg />} />);
    const cls = screen.getByRole('button').className;
    expect(cls).toContain('h-8');
    expect(cls).toContain('w-8');
    expect(cls).not.toContain('h-7');
  });

  it('has a frameless ghost variant', () => {
    render(<IconButton label="Back" variant="ghost" icon={<svg />} />);
    const cls = screen.getByRole('button').className;
    expect(cls).not.toContain('border-border');
    expect(cls).toContain('bg-transparent');
  });

  it('fires onClick', () => {
    const onClick = vi.fn();
    render(<IconButton label="Back" icon={<svg />} onClick={onClick} />);
    fireEvent.click(screen.getByRole('button'));
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe('MenuButton', () => {
  it('is a ghost icon button labelled "More actions" with a 14px glyph', () => {
    const { container } = render(<MenuButton />);
    const btn = screen.getByRole('button', { name: 'More actions' });
    expect(btn.className).toContain('bg-transparent');
    const svg = container.querySelector('svg')!;
    expect(svg.getAttribute('width')).toBe('14');
  });

  it('takes a specific label', () => {
    render(<MenuButton label="Meeting options" />);
    expect(screen.getByRole('button', { name: 'Meeting options' })).toBeTruthy();
  });

  it('works as a DropdownMenuTrigger child', () => {
    render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <MenuButton label="Row options" />
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem>Rename</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
    const trigger = screen.getByRole('button', { name: 'Row options' });
    expect(trigger.getAttribute('aria-haspopup')).toBe('menu');
    // Radix opens on pointerdown (left button, no ctrl) — and on Enter from the keyboard.
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toBeTruthy();
  });
});
