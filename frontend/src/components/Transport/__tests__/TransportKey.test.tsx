import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { TransportKey } from '@/components/Transport/TransportKey';

describe('TransportKey', () => {
  it('is a button with the given accessible name and reflects lit state', () => {
    const onClick = vi.fn();
    render(<TransportKey fn="rec" lit legend="REC" aria-label="Recording" onClick={onClick} />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(btn);
    expect(onClick).toHaveBeenCalledTimes(1);
  });
  it('does not fire when disabled', () => {
    const onClick = vi.fn();
    render(<TransportKey fn="hold" legend="HOLD" aria-label="Pause recording" disabled onClick={onClick} />);
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));
    expect(onClick).not.toHaveBeenCalled();
  });

  it('a lit REC key lights its whole face with cream ink', () => {
    const { container } = render(<TransportKey fn="rec" lit legend="REC" aria-label="Recording" />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.className).toContain('bg-lamp-red');
    expect(btn.className).not.toContain('bg-key');
    expect(container.querySelector('svg')?.getAttribute('class')).toContain('fill-record-foreground');
    expect(screen.getByText('REC').className).toContain('text-record-foreground');
  });

  it('a lit HOLD key lights amber with dark ink', () => {
    const { container } = render(<TransportKey fn="hold" lit legend="HOLD" aria-label="Paused" />);
    const btn = screen.getByRole('button', { name: 'Paused' });
    expect(btn.className).toContain('bg-lamp-amber');
    expect(container.querySelector('svg')?.getAttribute('class')).toContain('fill-lamp-ink');
    expect(screen.getByText('HOLD').className).toContain('text-lamp-ink');
  });

  it('a dimmed lit key (REC while on HOLD) keeps the unlit face', () => {
    render(<TransportKey fn="rec" lit dim legend="REC" aria-label="Recording" />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.className).toContain('bg-key');
    expect(btn.className).not.toContain('bg-lamp-red');
  });

  it('a lit and disabled key is not dimmed — the lit face carries its own state', () => {
    render(<TransportKey fn="rec" lit disabled legend="REC" aria-label="Recording" />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.className).not.toContain('opacity-[0.45]');
  });

  it('a disabled key that is not lit is still dimmed', () => {
    render(<TransportKey fn="hold" disabled legend="HOLD" aria-label="Pause recording" />);
    const btn = screen.getByRole('button', { name: 'Pause recording' });
    expect(btn.className).toContain('opacity-[0.45]');
  });

  it('STOP is never lit and keeps the engraved treatment', () => {
    const { container } = render(<TransportKey fn="stop" lit legend="STOP" aria-label="Stop recording" />);
    const btn = screen.getByRole('button', { name: 'Stop recording' });
    expect(btn.className).toContain('bg-key');
    expect(container.querySelector('svg')?.getAttribute('class')).toContain('fill-engrave');
  });

  it('the key is tall enough to clear the lamp bar and centres its contents', () => {
    render(<TransportKey fn="rec" legend="REC" aria-label="Recording" />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.className).toContain('h-11');
    expect(btn.className).toContain('justify-center');
    expect(btn.className).toContain('pt-2');
    expect(btn.className).not.toContain('justify-end');
    expect(btn.className).not.toContain('pb-1.5');
  });
});
