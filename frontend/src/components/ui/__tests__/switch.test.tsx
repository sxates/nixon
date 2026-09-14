import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { Switch } from '@/components/ui/switch';

// specs/0057 Task 2 — the Nixon 2-position toggle. It is still a Radix switch
// (16 call sites and RecordingSettings.test.tsx query `role="switch"`); only the
// skin changes, so the behavioural contract is what we lock down here.
describe('Switch', () => {
  it('renders a switch role reflecting the unchecked state', () => {
    render(<Switch aria-label="Diarization" />);
    const el = screen.getByRole('switch', { name: 'Diarization' });
    expect(el.getAttribute('data-state')).toBe('unchecked');
    expect(el.getAttribute('aria-checked')).toBe('false');
  });

  it('toggles data-state on click and reports the new value', () => {
    const onCheckedChange = vi.fn();
    render(<Switch aria-label="Diarization" onCheckedChange={onCheckedChange} />);
    const el = screen.getByRole('switch');
    fireEvent.click(el);
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(el.getAttribute('data-state')).toBe('checked');
  });

  it('honours the controlled checked prop and disabled', () => {
    render(<Switch aria-label="Diarization" checked disabled onCheckedChange={vi.fn()} />);
    const el = screen.getByRole('switch');
    expect(el.getAttribute('data-state')).toBe('checked');
    expect(el.hasAttribute('disabled')).toBe(true);
  });

  it('wears the 2-position rectangular track and sliding thumb', () => {
    const { container } = render(<Switch aria-label="Diarization" />);
    const root = screen.getByRole('switch');
    expect(root.className).toContain('h-[18px]');
    expect(root.className).toContain('w-8');
    expect(root.className).toContain('rounded-[3px]');
    expect(root.className).toContain('bg-well');
    expect(root.className).toContain('data-[state=checked]:bg-brand/25');
    const thumb = container.querySelector('[data-state] span, [data-state] > span');
    expect(thumb).toBeTruthy();
    expect(thumb!.className).toContain('data-[state=checked]:translate-x-[14px]');
    expect(thumb!.className).toContain('rounded-[2px]');
  });

  it('merges a caller className onto the track', () => {
    render(<Switch aria-label="Diarization" className="ml-2" />);
    expect(screen.getByRole('switch').className).toContain('ml-2');
  });
});
