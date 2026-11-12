import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Button } from '@/components/ui/button';

// docs/design-system.md §Component catalog — the one text button. Wave 0a aligned its
// base with the hand-rolled copies (semibold, ring-2) so they have no reason to exist.
describe('Button', () => {
  it('is semibold with a 2px focus ring and the 3px control radius', () => {
    render(<Button>Save</Button>);
    const el = screen.getByRole('button', { name: 'Save' });
    expect(el.className).toContain('font-semibold');
    expect(el.className).not.toContain('font-medium');
    expect(el.className).toContain('focus-visible:ring-2');
    expect(el.className).not.toContain('focus-visible:ring-1');
    expect(el.className).toContain('rounded-md');
  });

  it('keeps rounded-md at every size', () => {
    for (const size of ['xs', 'sm', 'default', 'lg', 'icon', 'icon-sm'] as const) {
      const { unmount } = render(<Button size={size}>x</Button>);
      expect(screen.getByRole('button').className).toContain('rounded-md');
      unmount();
    }
  });

  it('has a dashed "+ Add" variant that lights the brand edge on hover', () => {
    render(<Button variant="dashed">Add participant</Button>);
    const cls = screen.getByRole('button').className;
    expect(cls).toContain('border-dashed');
    expect(cls).toContain('text-muted-foreground');
    expect(cls).toContain('hover:border-brand-edge');
    expect(cls).toContain('hover:text-brand');
  });

  it('has a REC-red record variant whose hover comes from the variant', () => {
    render(<Button variant="record">Start recording</Button>);
    const cls = screen.getByRole('button').className;
    expect(cls).toContain('bg-record');
    expect(cls).toContain('text-record-foreground');
    expect(cls).toContain('hover:bg-record/90');
  });

  it('has a 28px icon-sm size', () => {
    render(<Button size="icon-sm" aria-label="Close" />);
    const cls = screen.getByRole('button', { name: 'Close' }).className;
    expect(cls).toContain('h-7');
    expect(cls).toContain('w-7');
  });
});

// A disabled lamp goes dark; it is never a dimmed lamp. `opacity-50` over amber kept the
// disabled onboarding Continue / Ask reading as a lit key in Deck.
describe('Button disabled lamps', () => {
  for (const variant of ['brand', 'record'] as const) {
    it(`${variant} goes dark when disabled`, () => {
      render(
        <Button variant={variant} disabled>
          Go
        </Button>,
      );
      const cls = screen.getByRole('button').className;
      expect(cls).toContain('disabled:bg-muted');
      expect(cls).toContain('disabled:text-muted-foreground');
      expect(cls).toContain('disabled:shadow-none');
      expect(cls).toContain('disabled:opacity-100');
      // twMerge drops the base dimming, so the dark key is not ALSO half-transparent.
      expect(cls).not.toContain('disabled:opacity-50');
    });
  }

  it('other variants keep the base dimming', () => {
    render(
      <Button variant="outline" disabled>
        Go
      </Button>,
    );
    expect(screen.getByRole('button').className).toContain('disabled:opacity-50');
  });
});
