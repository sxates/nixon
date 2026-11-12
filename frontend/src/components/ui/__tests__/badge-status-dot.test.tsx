import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Badge, BADGE_TONES } from '@/components/ui/badge';
import { StatusDot } from '@/components/ui/status-dot';

describe('Badge', () => {
  it('is an 11px semibold 2px chip, muted by default', () => {
    render(<Badge>Default</Badge>);
    const b = screen.getByText('Default');
    expect(b).toHaveClass('text-caption', 'font-semibold', 'rounded-sm');
    expect(b).toHaveClass('bg-muted', 'text-muted-foreground');
    expect(b).toHaveAttribute('data-tone', 'muted');
  });

  it.each([
    ['brand', ['bg-brand-wash', 'text-brand']],
    ['success', ['bg-success-wash', 'text-success']],
    ['muted', ['bg-muted', 'text-muted-foreground']],
    ['destructive', ['bg-destructive-wash', 'text-destructive']],
    ['outline', ['border', 'border-border', 'bg-transparent']],
  ] as const)('tone %s uses the named washes', (tone, classes) => {
    render(<Badge tone={tone}>x</Badge>);
    expect(screen.getByText('x')).toHaveClass(...classes);
  });

  it('never uses an ad-hoc opacity tint', () => {
    for (const cls of Object.values(BADGE_TONES)) expect(cls).not.toMatch(/\/\d+/);
  });

  it('forwards props and merges className', () => {
    render(<Badge tone="brand" title="Active calendar" className="ml-2">Active</Badge>);
    const b = screen.getByTitle('Active calendar');
    expect(b).toHaveClass('ml-2', 'bg-brand-wash');
  });
});

describe('StatusDot', () => {
  it('is an 8px round dot, hidden from assistive tech by default', () => {
    const { container } = render(<StatusDot tone="success" />);
    const dot = container.firstElementChild!;
    expect(dot).toHaveClass('h-2', 'w-2', 'rounded-full', 'bg-success');
    expect(dot).toHaveAttribute('aria-hidden', 'true');
    expect(dot).not.toHaveAttribute('role');
  });

  it.each([
    [6, ['h-1.5', 'w-1.5']],
    [8, ['h-2', 'w-2']],
    [10, ['h-2.5', 'w-2.5']],
  ] as const)('size %i', (size, classes) => {
    const { container } = render(<StatusDot size={size} tone="brand" />);
    expect(container.firstElementChild).toHaveClass(...classes);
  });

  it.each([
    ['brand', 'bg-brand'],
    ['record', 'bg-record'],
    ['success', 'bg-success'],
    ['destructive', 'bg-destructive'],
    ['muted', 'bg-engrave'],
  ] as const)('tone %s → %s', (tone, cls) => {
    const { container } = render(<StatusDot tone={tone} />);
    expect(container.firstElementChild).toHaveClass(cls);
  });

  it('takes a speaker colour class instead of a tone', () => {
    const { container } = render(<StatusDot size={10} className="bg-chart-3" />);
    const dot = container.firstElementChild!;
    expect(dot).toHaveClass('bg-chart-3', 'h-2.5');
    expect(dot.className).not.toMatch(/bg-(brand|record|success|engrave)/);
  });

  it('becomes a named image with a label', () => {
    render(<StatusDot tone="record" label="Recording" />);
    expect(screen.getByRole('img', { name: 'Recording' })).not.toHaveAttribute('aria-hidden');
  });
});
