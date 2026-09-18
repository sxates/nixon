import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { LampDot } from '@/components/Transport/LampDot';

describe('LampDot', () => {
  it('lit tones use the lamp tokens, not the text tokens', () => {
    render(<LampDot tone="amber" label="Paused" />);
    const amber = screen.getByRole('img', { name: 'Paused' });
    expect(amber.className).toContain('bg-lamp-amber');
    expect(amber.className).toContain('--lamp-amber');
    expect(amber.className).not.toContain('bg-brand');
  });

  it('the red lamp uses --lamp-red', () => {
    render(<LampDot tone="red" label="Recording" />);
    const red = screen.getByRole('img', { name: 'Recording' });
    expect(red.className).toContain('bg-lamp-red');
    expect(red.className).not.toContain('bg-record ');
  });

  it('an unlit lamp is named "<label> off" and carries no lamp token', () => {
    render(<LampDot tone="off" label="Queue" />);
    const off = screen.getByRole('img', { name: 'Queue off' });
    expect(off.className).toContain('bg-border');
    expect(off.className).not.toContain('lamp-');
  });

  it('a decorative lamp is hidden from the accessibility tree', () => {
    render(<LampDot tone="amber" label="Queue" decorative />);
    expect(screen.queryByRole('img')).toBeNull();
  });
});
