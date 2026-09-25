import { describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import { PersonAvatar, initials } from '@/components/People/PersonAvatar';

// specs/0056 W6 — People avatars render the cached Google directory photo when the person
// carries a `photoDataUri`, and fall back to the colored initials chip otherwise (or when the
// image fails to load). Shared by the directory row (sm) and the person page header (lg).

const URI = 'data:image/png;base64,iVBORw0KGgo=';

describe('PersonAvatar (specs/0056 W6)', () => {
  it('renders an <img> with the data: URI when a photo is present', () => {
    const { container } = render(
      <PersonAvatar name="Priya Patel" photoDataUri={URI} size="sm" colorClass="bg-chart-1" />,
    );
    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    expect(img!.getAttribute('src')).toBe(URI);
    expect(img!.getAttribute('alt')).toBe('');
    expect(container.textContent).not.toContain('PP');
  });

  it('renders initials when there is no photo', () => {
    const { container } = render(
      <PersonAvatar name="Priya Patel" photoDataUri={null} size="lg" colorClass="bg-chart-4" />,
    );
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('PP');
    // The caller's palette class is applied to the initials chip.
    expect(container.firstElementChild!.className).toMatch(/bg-chart-4/);
  });

  it('falls back to initials after the image errors', () => {
    const { container } = render(
      <PersonAvatar name="Priya Patel" photoDataUri={URI} size="sm" colorClass="bg-chart-1" />,
    );
    fireEvent.error(container.querySelector('img')!);
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('PP');
  });

  it('sizes the chip per variant', () => {
    const sm = render(
      <PersonAvatar name="A" photoDataUri={null} size="sm" colorClass="bg-chart-1" />,
    );
    expect(sm.container.firstElementChild!.className).toMatch(/h-8 w-8/);
    const lg = render(
      <PersonAvatar name="A" photoDataUri={null} size="lg" colorClass="bg-chart-1" />,
    );
    expect(lg.container.firstElementChild!.className).toMatch(/h-14 w-14/);
  });

  it('tries again when the photo changes after a failed load', () => {
    const OTHER = 'data:image/png;base64,T1RIRVI=';
    const { container, rerender } = render(
      <PersonAvatar name="Priya Patel" photoDataUri={URI} size="xs" colorClass="bg-chart-1" />,
    );
    fireEvent.error(container.querySelector('img')!);
    expect(container.querySelector('img')).toBeNull();
    rerender(<PersonAvatar name="Priya Patel" photoDataUri={OTHER} size="xs" colorClass="bg-chart-1" />);
    expect(container.querySelector('img')?.getAttribute('src')).toBe(OTHER);
  });

  it('renders the given fallback (the legend dot) instead of initials when the photo fails', () => {
    const dot = <span data-testid="dot" />;
    const { container, getByTestId } = render(
      <PersonAvatar name="Priya Patel" photoDataUri={URI} size="xxs" colorClass="bg-chart-1" fallback={dot} />,
    );
    fireEvent.error(container.querySelector('img')!);
    expect(getByTestId('dot')).toBeTruthy();
    expect(container.textContent).not.toContain('PP');
  });

  it('initials(): first + last, single name, and empty', () => {
    expect(initials('Priya Sharma')).toBe('PS');
    expect(initials('  priya  ')).toBe('P');
    expect(initials('Ana Maria de Souza')).toBe('AS');
    expect(initials('   ')).toBe('?');
  });
});
