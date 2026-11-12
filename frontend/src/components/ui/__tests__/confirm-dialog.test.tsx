import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { ConfirmDialog } from '@/components/ui/confirm-dialog';

const base = {
  open: true,
  title: 'Clear all voiceprints',
  body: 'This can’t be undone.',
  confirmLabel: 'Clear all voiceprints',
};

describe('ConfirmDialog', () => {
  it('is a dialog named by its title and described by its body', () => {
    render(<ConfirmDialog {...base} onOpenChange={() => {}} onConfirm={() => {}} />);
    const dialog = screen.getByRole('dialog', { name: 'Clear all voiceprints' });
    expect(dialog).toHaveAccessibleDescription('This can’t be undone.');
  });

  it('destructive tone (default) fills the confirm destructive; brand tone uses brand', () => {
    const { rerender } = render(<ConfirmDialog {...base} onOpenChange={() => {}} onConfirm={() => {}} />);
    expect(screen.getByRole('button', { name: 'Clear all voiceprints' })).toHaveClass('bg-destructive');
    rerender(
      <ConfirmDialog {...base} confirmLabel="Move recordings" tone="brand" onOpenChange={() => {}} onConfirm={() => {}} />,
    );
    const confirm = screen.getByRole('button', { name: 'Move recordings' });
    expect(confirm).toHaveClass('bg-brand');
    expect(confirm).not.toHaveClass('bg-destructive');
  });

  it('confirm calls onConfirm; Cancel closes', () => {
    const onConfirm = vi.fn();
    const onOpenChange = vi.fn();
    render(<ConfirmDialog {...base} cancelLabel="Keep them" onOpenChange={onOpenChange} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByRole('button', { name: 'Clear all voiceprints' }));
    expect(onConfirm).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: 'Keep them' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('busy: spinner, both buttons disabled, and Escape cannot close it', () => {
    const onOpenChange = vi.fn();
    render(<ConfirmDialog {...base} busy onOpenChange={onOpenChange} onConfirm={() => {}} />);
    const confirm = screen.getByRole('button', { name: 'Clear all voiceprints' });
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveAttribute('aria-busy', 'true');
    expect(confirm.querySelector('svg.animate-spin')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it('confirmDisabled disables only the confirm', () => {
    render(<ConfirmDialog {...base} confirmDisabled onOpenChange={() => {}} onConfirm={() => {}} />);
    expect(screen.getByRole('button', { name: 'Clear all voiceprints' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).not.toBeDisabled();
  });
});
