import { describe, it, expect } from 'vitest';

import { cn } from '@/lib/utils';

// tailwind-merge only knows Tailwind's built-in scale. Without the extension in
// lib/utils.ts it reads `text-caption` as a COLOUR and drops it next to a real colour.
describe('cn() with the design-system tokens', () => {
  it('keeps a named size next to a text colour', () => {
    expect(cn('text-caption', 'text-muted-foreground')).toBe('text-caption text-muted-foreground');
    expect(cn('text-body text-foreground')).toBe('text-body text-foreground');
  });

  it('lets a named size override a built-in one (and vice versa)', () => {
    expect(cn('text-sm', 'text-caption')).toBe('text-caption');
    expect(cn('text-title', 'text-xs')).toBe('text-xs');
  });

  it('treats shadow-card as a shadow', () => {
    expect(cn('shadow-card', 'shadow-none')).toBe('shadow-none');
  });
});
