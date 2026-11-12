import type * as React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { PersonOption, PersonPicker, matchesPersonQuery } from '@/components/ui/person-picker';

describe('PersonOption', () => {
  it('shows the name, a muted suffix and the email caption', () => {
    const onSelect = vi.fn();
    render(<PersonOption name="Priya Patel" suffix="(you)" email="priya@example.com" onSelect={onSelect} />);
    const btn = screen.getByRole('button', { name: /Priya Patel/ });
    expect(screen.getByText('Priya Patel')).toHaveClass('font-medium');
    expect(screen.getByText('(you)')).toHaveClass('text-muted-foreground');
    expect(screen.getByText('priya@example.com')).toHaveClass('text-caption', 'text-muted-foreground');
    expect(btn).toHaveClass('focus-visible:ring-2', 'rounded-md');
    fireEvent.click(btn);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('omits the email line when there is none and marks the current choice', () => {
    render(<PersonOption name="Bo" onSelect={() => {}} selected />);
    const btn = screen.getByRole('button', { name: 'Bo' });
    expect(btn).toHaveAttribute('aria-pressed', 'true');
    expect(btn).toHaveClass('bg-brand-wash');
    expect(btn.querySelector('.text-caption')).toBeNull();
  });
});

describe('matchesPersonQuery', () => {
  it('matches name or email, case-insensitively; empty matches all', () => {
    expect(matchesPersonQuery('pri', 'Priya Patel')).toBe(true);
    expect(matchesPersonQuery('EXAMPLE', 'Bo', 'bo@example.com')).toBe(true);
    expect(matchesPersonQuery('zed', 'Bo', null)).toBe(false);
    expect(matchesPersonQuery('  ', 'Bo')).toBe(true);
  });
});

describe('PersonPicker', () => {
  const setup = (overrides: Partial<React.ComponentProps<typeof PersonPicker>> = {}) => {
    const onQueryChange = vi.fn();
    const onSubmit = vi.fn();
    render(
      <PersonPicker
        query="pr"
        onQueryChange={onQueryChange}
        onSubmit={onSubmit}
        placeholder="Search or type a name…"
        groups={[
          {
            label: 'Attendees',
            options: [<PersonOption key="p" name="Priya" email="p@x.com" onSelect={() => {}} />],
          },
          { label: 'People', options: [], emptyMessage: 'No people match “pr”.' },
          { label: 'Hidden', options: [] },
        ]}
        {...overrides}
      />,
    );
    return { onQueryChange, onSubmit };
  };

  it('a labelled search field drives the query; Enter submits', () => {
    const { onQueryChange, onSubmit } = setup();
    const input = screen.getByRole('textbox', { name: 'Search or type a name…' });
    fireEvent.change(input, { target: { value: 'pri' } });
    expect(onQueryChange).toHaveBeenCalledWith('pri');
    fireEvent.submit(input.closest('form')!);
    expect(onSubmit).toHaveBeenCalledOnce();
  });

  it('renders named groups, an empty message, and hides empty groups without one', () => {
    setup();
    expect(screen.getByRole('group', { name: 'Attendees' })).toContainElement(
      screen.getByRole('button', { name: /Priya/ }),
    );
    expect(screen.getByRole('group', { name: 'People' })).toHaveTextContent('No people match “pr”.');
    expect(screen.queryByRole('group', { name: 'Hidden' })).toBeNull();
  });

  it('puts before-rows and an input action in place', () => {
    setup({ before: <button type="button">Me</button>, inputAction: <button type="submit">Assign</button> });
    expect(screen.getByRole('button', { name: 'Me' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Assign' }).closest('form')).not.toBeNull();
  });
});
