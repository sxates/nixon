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
});
