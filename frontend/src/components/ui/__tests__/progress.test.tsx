import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';

import { Progress } from '@/components/ui/progress';

describe('Progress', () => {
  it('is a 6px muted track with an amber (brand) fill', () => {
    const { container } = render(<Progress value={40} />);
    const root = container.querySelector('[role="progressbar"]')!;
    expect(root.className).toContain('h-1.5');
    expect(root.className).toContain('rounded-full');
    expect(root.className).toContain('bg-muted');
    expect(root.className).not.toContain('bg-primary');
    const fill = root.firstElementChild as HTMLElement;
    expect(fill.className).toContain('bg-brand');
    expect(fill.style.transform).toBe('translateX(-60%)');
  });

  it('reports its value to assistive tech', () => {
    const { container } = render(<Progress value={40} />);
    expect(container.querySelector('[role="progressbar"]')!.getAttribute('aria-valuenow')).toBe('40');
  });

  it('takes a tone', () => {
    const { container } = render(<Progress value={100} tone="success" />);
    const fill = container.querySelector('[role="progressbar"]')!.firstElementChild!;
    expect(fill.className).toContain('bg-success');
    expect(fill.className).not.toContain('bg-brand');
  });
});
