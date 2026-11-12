import { describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/react';

import { Avatar, AvatarStack, initials } from '@/components/ui/avatar';
import { initials as dayAgendaInitials } from '@/lib/day-agenda';

const URI = 'data:image/png;base64,iVBORw0KGgo=';

describe('initials()', () => {
  it('first + last, single word, email local part, empty', () => {
    expect(initials('Priya Sharma')).toBe('PS');
    expect(initials('Ana Maria de Souza')).toBe('AS');
    expect(initials('  priya  ')).toBe('P');
    expect(initials('jordan.lee@example.com')).toBe('JL');
    expect(initials('   ')).toBe('?');
  });

  it('is the one helper (the old import paths re-export it)', () => {
    expect(dayAgendaInitials).toBe(initials);
  });
});

describe('Avatar', () => {
  it('draws initials on the hashed speaker colour by default, 32px', () => {
    const { container } = render(<Avatar name="Priya Patel" />);
    const el = container.firstElementChild!;
    expect(el.textContent).toBe('PP');
    expect(el).toHaveClass('h-8', 'w-8', 'rounded-full');
    expect(el.className).toMatch(/bg-chart-\d/);
    expect(el).toHaveAttribute('aria-hidden', 'true');
  });

  it.each([
    [16, 'h-4'],
    [22, 'h-[22px]'],
    [24, 'h-6'],
    [28, 'h-7'],
    [32, 'h-8'],
    [56, 'h-14'],
  ] as const)('size %i', (size, cls) => {
    const { container } = render(<Avatar name="A B" size={size} />);
    expect(container.firstElementChild).toHaveClass(cls);
    expect(container.firstElementChild!.className).not.toMatch(/text-\[/);
  });

  it('uses foreground ink on the light muted chip', () => {
    const { container } = render(<Avatar name="A" colorClass="bg-muted" />);
    expect(container.firstElementChild).toHaveClass('text-foreground');
  });

  it('shows the photo, then falls back to initials when it fails, and retries a new photo', () => {
    const { container, rerender } = render(<Avatar name="Priya Patel" photo={URI} />);
    const img = container.querySelector('img')!;
    expect(img.getAttribute('src')).toBe(URI);
    expect(img.getAttribute('alt')).toBe('');
    fireEvent.error(img);
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('PP');
    rerender(<Avatar name="Priya Patel" photo="data:image/png;base64,T1RIRVI=" />);
    expect(container.querySelector('img')).not.toBeNull();
  });

  it('renders the fallback node instead of initials', () => {
    const { getByTestId, container } = render(
      <Avatar name="Priya" fallback={<span data-testid="dot" />} />,
    );
    expect(getByTestId('dot')).toBeTruthy();
    expect(container.textContent).toBe('');
  });
});

describe('AvatarStack', () => {
  const people = [
    { name: 'Ada Example', colorKey: 'ada@example.com', title: 'Ada Example · ada@example.com' },
    { name: 'Bo Chen' },
    { name: 'Cy Dee' },
    { name: 'Di Eve' },
  ];

  it('draws up to max, and nothing when empty', () => {
    const { container, rerender } = render(<AvatarStack people={people} max={3} />);
    expect(container.querySelectorAll('[data-avatar-stack] > *')).toHaveLength(3);
    rerender(<AvatarStack people={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('overlaps by 4px with a card ring so a neighbour never covers the initials', () => {
    const { container } = render(<AvatarStack people={people} size={22} />);
    const discs = Array.from(container.querySelectorAll('[data-avatar-stack] > *'));
    expect(discs[0]).not.toHaveClass('-ml-1');
    for (const d of discs.slice(1)) {
      expect(d).toHaveClass('-ml-1', 'border-2', 'border-card');
      // The old 7px overlap clipped the second initial on 22px discs.
      expect(d.className).not.toContain('-ml-[7px]');
    }
    expect(discs[0]).toHaveClass('h-[22px]', 'text-micro');
  });

  it('keeps each person’s tooltip', () => {
    const { getByTitle } = render(<AvatarStack people={people} />);
    expect(getByTitle('Ada Example · ada@example.com').textContent).toBe('AE');
  });
});
